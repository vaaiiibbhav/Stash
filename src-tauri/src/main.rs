// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::{engine::general_purpose, Engine as _};
use screenshots::Screen;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tauri::menu::{MenuBuilder, MenuItemBuilder};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WindowEvent};

use windows::core::{s, PCSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, FILETIME, HANDLE, HWND, LPARAM, NTSTATUS};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, GetProcessTimes, OpenProcess, QueryFullProcessImageNameW,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SUSPEND_RESUME,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowInfo, GetWindowTextW, GetWindowThreadProcessId, ShowWindowAsync, SW_HIDE,
    SW_SHOW, WINDOWINFO, WS_VISIBLE,
};

/// A window we stashed: the window to re-show and the process to resume.
/// `start_time` is the owning process's creation timestamp, used to tell a
/// live process apart from an unrelated one that merely inherited its PID
/// (Windows recycles PIDs aggressively). `title` is carried for crash
/// recovery display only — nothing in the live suspend/resume path reads it.
#[derive(Clone)]
struct Handle {
    hwnd: HWND,
    pid: u32,
    start_time: u64,
    title: String,
}

/// Shared store of stashed windows, guarded by a mutex.
struct Handles(Mutex<Vec<Handle>>);

/// On-disk mirror of `Handle`, written to the app's local data dir every time
/// a stash succeeds and cleared every time a restore fully succeeds. Its only
/// purpose is crash recovery: if Stash itself is killed (crash, force-kill,
/// logoff) while apps are stashed, the live `Handles` mutex dies with it, but
/// this file survives, so the next launch can detect and offer to resume the
/// orphaned processes. `hwnd` is stored as the raw pointer value (`isize`)
/// since `HWND` itself isn't `Serialize`.
#[derive(Serialize, Deserialize, Clone)]
struct PersistedHandle {
    pid: u32,
    hwnd: isize,
    #[serde(default)]
    start_time: u64,
    title: String,
}

fn stash_file_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = match app.path().app_local_data_dir() {
        Ok(dir) => dir,
        Err(e) => {
            log::error!("no app-local-data dir available: {e}");
            return None;
        }
    };
    if let Err(e) = fs::create_dir_all(&dir) {
        log::error!("could not create {}: {e}", dir.display());
        return None;
    }
    Some(dir.join("stashed-session.json"))
}

/// Writes the crash-recovery record atomically: serialize to a sibling
/// temp file, then rename over the real path. `fs::rename` replaces the
/// destination on Windows, and a rename is atomic, so a crash (or a logoff
/// killing us mid-write) can leave behind a stale record or a stray temp
/// file, but never a half-written — and therefore unparseable — record.
///
/// Best-effort throughout: the windows are already suspended and hidden by
/// the time this runs, so a persistence failure must never fail the stash.
fn persist_handles(app: &AppHandle, handles: &[Handle]) {
    let Some(path) = stash_file_path(app) else {
        return;
    };

    let persisted: Vec<PersistedHandle> = handles
        .iter()
        .map(|h| PersistedHandle {
            pid: h.pid,
            hwnd: h.hwnd.0,
            start_time: h.start_time,
            title: h.title.clone(),
        })
        .collect();

    let json = match serde_json::to_string(&persisted) {
        Ok(json) => json,
        Err(e) => {
            log::error!("could not serialize the stash record: {e}");
            return;
        }
    };

    let tmp = path.with_extension("json.tmp");
    if let Err(e) = fs::write(&tmp, json) {
        log::error!("could not write {}: {e}", tmp.display());
        return;
    }
    if let Err(e) = fs::rename(&tmp, &path) {
        log::error!("could not replace {}: {e}", path.display());
        let _ = fs::remove_file(&tmp);
        return;
    }
    log::info!("recorded {} stashed window(s) for crash recovery", handles.len());
}

fn clear_persisted(app: &AppHandle) {
    if let Some(path) = stash_file_path(app) {
        // NotFound is the expected steady state (nothing stashed); anything
        // else is worth knowing about.
        if let Err(e) = fs::remove_file(&path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::warn!("could not remove {}: {e}", path.display());
            }
        }
    }
}

/// Moves an unparseable record aside instead of deleting it, so the next
/// launch starts clean while the bad file is still available to inspect.
fn quarantine_corrupt(path: &Path) {
    let corrupt = path.with_extension("json.corrupt");
    match fs::rename(path, &corrupt) {
        Ok(()) => log::warn!("moved unreadable stash record to {}", corrupt.display()),
        Err(e) => {
            log::error!("could not quarantine {}: {e}", path.display());
            let _ = fs::remove_file(path);
        }
    }
}

/// Reads the crash-recovery record. A missing file is the normal steady
/// state and is silent; a file that exists but cannot be read or parsed is
/// logged and quarantined rather than silently treated as "nothing was
/// stashed", which would strand suspended processes with no way back.
fn read_persisted(app: &AppHandle) -> Vec<PersistedHandle> {
    let Some(path) = stash_file_path(app) else {
        return Vec::new();
    };

    let raw = match fs::read_to_string(&path) {
        Ok(raw) => raw,
        Err(e) => {
            if e.kind() != std::io::ErrorKind::NotFound {
                log::error!("could not read {}: {e}", path.display());
                quarantine_corrupt(&path);
            }
            return Vec::new();
        }
    };

    match serde_json::from_str::<Vec<PersistedHandle>>(&raw) {
        Ok(entries) => entries,
        Err(e) => {
            log::error!(
                "stash record at {} is malformed ({e}); it will be quarantined",
                path.display()
            );
            quarantine_corrupt(&path);
            Vec::new()
        }
    }
}

/// One window discovered during enumeration. Defined once at module scope so
/// the `EnumWindows` callback and `get_apps` agree on the exact layout that the
/// raw `LPARAM` pointer is cast to.
struct WindowEntry {
    title: String,
    hwnd: HWND,
}

// ---------------------------------------------------------------------------
// Process suspension
//
// We suspend/resume whole processes via ntdll's `NtSuspendProcess` /
// `NtResumeProcess`. They are undocumented but stable across every supported
// Windows release, and calling them directly removes the previous dependency on
// bundling the Sysinternals `pssuspend` sidecar (which we are not licensed to
// redistribute). ntdll.dll is mapped into every process, so we resolve the
// entry points lazily with GetProcAddress rather than linking against it.
// ---------------------------------------------------------------------------

type NtProcessControl = unsafe extern "system" fn(HANDLE) -> NTSTATUS;

fn nt_process_control(name: PCSTR) -> Option<NtProcessControl> {
    unsafe {
        let ntdll = GetModuleHandleA(s!("ntdll.dll")).ok()?;
        let proc = GetProcAddress(ntdll, name)?;
        Some(std::mem::transmute::<unsafe extern "system" fn() -> isize, NtProcessControl>(proc))
    }
}

/// Suspends (`suspend = true`) or resumes (`suspend = false`) every thread of
/// `pid`. Returns `Err` with a human-readable reason if the process cannot be
/// opened or the Nt call reports failure — the caller decides what to do, so a
/// single inaccessible (e.g. elevated) process never aborts the whole batch.
fn set_process_suspended(pid: u32, suspend: bool) -> Result<(), String> {
    let label = if suspend {
        "NtSuspendProcess"
    } else {
        "NtResumeProcess"
    };
    let entry: PCSTR = if suspend {
        s!("NtSuspendProcess")
    } else {
        s!("NtResumeProcess")
    };

    unsafe {
        let handle = OpenProcess(PROCESS_SUSPEND_RESUME, false, pid)
            .map_err(|e| format!("OpenProcess({pid}) failed: {e}"))?;

        let control = match nt_process_control(entry) {
            Some(func) => func,
            None => {
                let _ = CloseHandle(handle);
                return Err(format!("could not resolve {label} in ntdll.dll"));
            }
        };

        let status = control(handle);
        let _ = CloseHandle(handle);

        if status.is_ok() {
            Ok(())
        } else {
            Err(format!(
                "{label} failed for pid {pid}: NTSTATUS 0x{:08X}",
                status.0
            ))
        }
    }
}

/// Best-effort base file name (e.g. `explorer.exe`) for a pid. `None` if the
/// process cannot be queried; unlike the old `GetWindowModuleFileNameA` this
/// works for windows owned by other processes.
fn process_exe_name(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut buf = [0u16; 260]; // MAX_PATH
        let mut size = buf.len() as u32;
        let query = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut size,
        );
        let _ = CloseHandle(handle);

        if !query.as_bool() {
            return None;
        }
        let full = String::from_utf16_lossy(&buf[..size as usize]);
        full.rsplit(['\\', '/']).next().map(str::to_string)
    }
}

/// The process's creation time as a raw FILETIME (100ns ticks). Together with
/// the PID this forms an identity that survives PID recycling: Windows can
/// hand pid 4242 to a new process, but not with the same creation timestamp.
fn process_start_time(pid: u32) -> Option<u64> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;

        let mut creation = FILETIME::default();
        let mut exit = FILETIME::default();
        let mut kernel = FILETIME::default();
        let mut user = FILETIME::default();
        let ok = GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user);
        let _ = CloseHandle(handle);

        if !ok.as_bool() {
            return None;
        }
        Some(((creation.dwHighDateTime as u64) << 32) | creation.dwLowDateTime as u64)
    }
}

/// True if the process behind `pid` is still the exact one we stashed, rather
/// than an unrelated process that was later assigned the same PID. Records
/// written before start-time tracking existed carry `start_time == 0`; those
/// fall back to a plain liveness check.
fn is_same_process(pid: u32, recorded_start_time: u64) -> bool {
    match process_start_time(pid) {
        Some(current) => recorded_start_time == 0 || current == recorded_start_time,
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Captures the primary screen and returns it as a base64-encoded PNG.
#[tauri::command]
fn screen_shot() -> Result<String, String> {
    let screens = Screen::all().map_err(|e| format!("failed to enumerate screens: {e}"))?;
    let screen = screens
        .first()
        .ok_or_else(|| "no screens available to capture".to_string())?;
    let image = screen
        .capture()
        .map_err(|e| format!("screen capture failed: {e}"))?;
    Ok(general_purpose::STANDARD.encode(image.buffer()))
}

// System surfaces we never stash, matched loosely by window title. Our own
// window is excluded by PID (in `get_apps`), so it is deliberately absent
// here.
const TITLE_ALLOWLIST: [&str; 2] = ["Task Manager", "Windows Security"];
// Suspending these would freeze the Windows shell (taskbar, desktop, tray),
// including our own restore-from-tray path. Matched by executable name.
const EXE_BLOCKLIST: [&str; 1] = ["explorer.exe"];

/// True if `title` loosely (case-insensitive substring) matches one of the
/// always-visible system surfaces we never stash.
fn is_allowlisted_title(title: &str) -> bool {
    let lower = title.to_lowercase();
    TITLE_ALLOWLIST
        .iter()
        .any(|allowed| lower.contains(&allowed.to_lowercase()))
}

/// True if `exe` (a bare file name, e.g. "explorer.exe") is one we must never
/// suspend, because doing so would freeze the Windows shell itself.
fn is_blocklisted_exe(exe: &str) -> bool {
    EXE_BLOCKLIST
        .iter()
        .any(|blocked| exe.eq_ignore_ascii_case(blocked))
}

/// Enumerates visible top-level windows, suspends their owning processes, and
/// hides them. Returns a short comma-separated preview of up to three window
/// titles and the number of processes actually suspended.
#[tauri::command]
fn get_apps(app: AppHandle, handles: State<Handles>) -> Result<(String, usize), String> {
    let own_pid = unsafe { GetCurrentProcessId() };

    let mut discovered: Vec<WindowEntry> = Vec::new();
    unsafe {
        EnumWindows(
            Some(enum_window),
            LPARAM(&mut discovered as *mut Vec<WindowEntry> as _),
        )
        .ok()
        .map_err(|e| format!("EnumWindows failed: {e}"))?;
    }

    let mut store = handles
        .0
        .lock()
        .map_err(|_| "window store is poisoned".to_string())?;

    let mut preview: Vec<String> = Vec::new();
    let mut suspended = 0usize;

    for entry in &discovered {
        let mut pid: u32 = 0;
        unsafe { GetWindowThreadProcessId(entry.hwnd, Some(&mut pid)) };

        // Never touch our own process (would freeze the app mid-command) or a
        // window whose owner we could not identify.
        if pid == 0 || pid == own_pid {
            continue;
        }

        if is_allowlisted_title(&entry.title) {
            continue;
        }

        if let Some(exe) = process_exe_name(pid) {
            if is_blocklisted_exe(&exe) {
                continue;
            }
        }

        // Capture identity *before* suspending, so the crash-recovery record
        // can later tell this exact process from a PID-recycled impostor.
        let start_time = process_start_time(pid).unwrap_or(0);

        // Suspend first; only hide the window once suspension has actually
        // succeeded, so we never leave a window hidden while its process runs.
        match set_process_suspended(pid, true) {
            Ok(()) => {
                let _ = unsafe { ShowWindowAsync(entry.hwnd, SW_HIDE) };
                store.push(Handle {
                    hwnd: entry.hwnd,
                    pid,
                    start_time,
                    title: clean_title(&entry.title),
                });
                suspended += 1;
                if preview.len() < 3 {
                    preview.push(clean_title(&entry.title));
                }
            }
            // Access-denied on elevated processes is expected: skip and carry on.
            Err(reason) => log::debug!("skipping window (pid {pid}): {reason}"),
        }
    }

    // Mirror the just-built store to disk so a crash/force-kill before the
    // next successful restore can still be recovered from on next launch.
    persist_handles(&app, store.as_slice());

    log::info!("stashed {suspended} process(es)");
    Ok((preview.join(", "), suspended))
}

/// Resumes and re-shows every stashed window. Exposed as a command; the tray
/// "Quit" path calls the shared implementation directly.
#[tauri::command]
fn restore(app: AppHandle, handles: State<Handles>) -> Result<(), String> {
    restore_all(&app, handles.inner())
}

/// Pulls every `Handle` out of the shared store in one lock, leaving it
/// empty. Draining (rather than cloning) guarantees a second stash/restore
/// cycle can never resume a stale, since-recycled PID or re-show a window we
/// no longer own. Split out from `restore_all` so the store-emptying
/// invariant can be unit tested without touching real Windows processes.
fn drain_handles(handles: &Handles) -> Result<Vec<Handle>, String> {
    let mut store = handles
        .0
        .lock()
        .map_err(|_| "window store is poisoned".to_string())?;
    Ok(std::mem::take(&mut *store))
}

/// Drains the window store and resumes/re-shows each entry. On full success,
/// also clears the on-disk crash-recovery record written by `get_apps` — a
/// partial failure deliberately leaves it in place, since the still-suspended
/// processes it names are exactly what `check_orphaned_stash` should keep
/// being able to offer to resume.
fn restore_all(app: &AppHandle, handles: &Handles) -> Result<(), String> {
    let drained = drain_handles(handles)?;

    let mut errors: Vec<String> = Vec::new();
    for handle in drained {
        // A PID we stashed may since have exited and had its number reused;
        // resuming that stranger would be an unrelated side effect.
        if !is_same_process(handle.pid, handle.start_time) {
            log::debug!("pid {} is gone or recycled; skipping", handle.pid);
            continue;
        }
        match set_process_suspended(handle.pid, false) {
            // Only re-show once the process is actually running again.
            Ok(()) => {
                let _ = unsafe { ShowWindowAsync(handle.hwnd, SW_SHOW) };
            }
            Err(reason) => errors.push(reason),
        }
    }

    if errors.is_empty() {
        clear_persisted(app);
        log::info!("restored every stashed window");
        Ok(())
    } else {
        log::error!("restore finished with {} error(s)", errors.len());
        Err(errors.join("; "))
    }
}

/// Reports whether a stash record was left on disk by a previous run that
/// didn't exit cleanly (crash, force-kill, logoff — the normal Quit and
/// Restore paths both clear it). Returned as `(found, preview, count)` rather
/// than an `Option`-shaped object: Tauri's IPC has no ambiguity serializing a
/// tuple, whereas an `Option<T>` at the JSON boundary would require the F#
/// side to trust that a JS `null` round-trips to `None` through Fable's raw
/// `invoke` — exactly the kind of untyped-boundary risk this project avoids
/// elsewhere. `count` only reflects entries whose process is still the one we
/// stashed; exited and PID-recycled entries are dropped.
#[tauri::command]
fn check_orphaned_stash(app: AppHandle) -> Result<(bool, String, usize), String> {
    let persisted = read_persisted(&app);
    if persisted.is_empty() {
        return Ok((false, String::new(), 0));
    }

    let alive: Vec<&PersistedHandle> = persisted
        .iter()
        .filter(|h| is_same_process(h.pid, h.start_time))
        .collect();

    if alive.is_empty() {
        // Every recorded process is already gone (closed manually, or reaped
        // by the OS); the stale record no longer describes anything real.
        log::info!("found a stash record but none of its processes are still alive");
        clear_persisted(&app);
        return Ok((false, String::new(), 0));
    }

    log::info!("found an orphaned stash with {} live process(es)", alive.len());

    let preview = alive
        .iter()
        .take(3)
        .map(|h| h.title.clone())
        .collect::<Vec<_>>()
        .join(", ");

    Ok((true, preview, alive.len()))
}

/// Resumes and re-shows every process recorded in a leftover stash file from
/// a previous run, then clears the record. Deliberately independent of the
/// live in-memory `Handles` store: a crash means that store was reset to
/// empty when this process started, so there is nothing to drain from it.
#[tauri::command]
fn resume_orphaned(app: AppHandle) -> Result<(), String> {
    let persisted = read_persisted(&app);

    let mut errors: Vec<String> = Vec::new();
    for entry in &persisted {
        // Skip anything that exited or whose PID now belongs to someone else.
        if !is_same_process(entry.pid, entry.start_time) {
            continue;
        }
        match set_process_suspended(entry.pid, false) {
            Ok(()) => {
                let _ = unsafe { ShowWindowAsync(HWND(entry.hwnd), SW_SHOW) };
            }
            Err(reason) => errors.push(reason),
        }
    }

    clear_persisted(&app);

    if errors.is_empty() {
        log::info!("resumed an orphaned stash");
        Ok(())
    } else {
        log::error!("resuming the orphaned stash hit {} error(s)", errors.len());
        Err(errors.join("; "))
    }
}

/// Trims a window title to a short, human-friendly label: for titles of the
/// form "Document — App" it keeps the part after the em dash.
fn clean_title(title: &str) -> String {
    match title.split_once('—') {
        Some((_, rest)) => rest.trim().to_string(),
        None => title.trim().to_string(),
    }
}

/// `EnumWindows` callback: collects visible, on-screen, titled windows into the
/// `Vec<WindowEntry>` passed via `lparam`. Must never panic — unwinding across
/// this `extern "system"` boundary would abort the process — so failures just
/// skip the window and continue enumeration.
extern "system" fn enum_window(window: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        let discovered = &mut *(lparam.0 as *mut Vec<WindowEntry>);

        let mut text = [0u16; 512];
        let len = GetWindowTextW(window, &mut text);
        if len <= 0 {
            return true.into();
        }
        let title = String::from_utf16_lossy(&text[..len as usize]);

        let mut info = WINDOWINFO {
            cbSize: std::mem::size_of::<WINDOWINFO>() as u32,
            ..Default::default()
        };
        if !GetWindowInfo(window, &mut info).as_bool() {
            return true.into();
        }

        let visible = info.dwStyle.contains(WS_VISIBLE);
        // Skip windows pinned to the exact top-left origin (0,0): these are
        // typically off-screen shell/host windows, not user apps.
        let on_screen = info.rcWindow.left != 0 || info.rcWindow.top != 0;

        if !title.is_empty() && visible && on_screen {
            discovered.push(WindowEntry { title, hwnd: window });
        }

        true.into()
    }
}

/// Brings the main window back into view. Every caller is a UI affordance
/// (tray click, tray "Show"), so a missing window is logged rather than
/// panicking the whole app out from under the user's stashed session.
fn show_main_window(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        log::error!("no 'main' window to show");
        return;
    };
    if let Err(e) = window.show() {
        log::warn!("could not show the main window: {e}");
    }
    if let Err(e) = window.set_focus() {
        log::warn!("could not focus the main window: {e}");
    }
}

/// Builds the tray icon, its menu, and their handlers. Done in code rather
/// than via `app.trayIcon` in tauri.conf.json so there is exactly one tray
/// icon and it owns its own menu wiring.
fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let show = MenuItemBuilder::with_id("show", "Show").build(app)?;
    let quit = MenuItemBuilder::with_id("quit", "Quit").build(app)?;
    let menu = MenuBuilder::new(app).items(&[&show, &quit]).build()?;

    let mut tray = TrayIconBuilder::new()
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "quit" => {
                // Resume everything we stashed before exiting, natively — no
                // fragile eval into the webview, and this actually exits.
                if let Some(handles) = app.try_state::<Handles>() {
                    if let Err(reason) = restore_all(app, handles.inner()) {
                        log::error!("restore-on-quit failed: {reason}");
                    }
                }
                app.exit(0);
            }
            other => log::debug!("unhandled tray menu id: {other}"),
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    // Reuse the window icon so the tray never renders blank; if the bundled
    // icon is somehow unavailable, a menu-only tray is still usable.
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    } else {
        log::warn!("no default window icon available for the tray");
    }

    tray.build(app)?;
    Ok(())
}

fn main() {
    tauri::Builder::default()
        .plugin(
            tauri_plugin_log::Builder::new()
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::LogDir {
                        file_name: Some("stash".into()),
                    },
                ))
                .target(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ))
                .level(log::LevelFilter::Info)
                .build(),
        )
        .setup(|app| {
            let handle = app.handle();

            match app.get_webview_window("main") {
                Some(window) => {
                    if let Err(e) = window.set_always_on_top(true) {
                        log::warn!("could not pin the window on top: {e}");
                    }
                }
                None => log::error!("no 'main' window found during setup"),
            }

            if let Err(e) = build_tray(handle) {
                // A tray failure is survivable — the window still works — but
                // it removes the restore-from-tray path, so shout about it.
                log::error!("could not build the tray icon: {e}");
            }

            log::info!("Stash {} started", env!("CARGO_PKG_VERSION"));
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Close means "get out of the way", not "quit" — quitting is
                // the tray's job, and it restores stashed apps on the way out.
                if let Err(e) = window.hide() {
                    log::warn!("could not hide the window on close: {e}");
                }
                api.prevent_close();
            }
        })
        .manage(Handles(Default::default()))
        .invoke_handler(tauri::generate_handler![
            get_apps,
            restore,
            screen_shot,
            check_orphaned_stash,
            resume_orphaned
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_title_keeps_part_after_em_dash() {
        assert_eq!(clean_title("readme.txt — Notepad"), "Notepad");
    }

    #[test]
    fn clean_title_trims_surrounding_whitespace_after_split() {
        assert_eq!(clean_title("doc —   Notepad  "), "Notepad");
    }

    #[test]
    fn clean_title_passes_through_titles_without_an_em_dash() {
        assert_eq!(clean_title("Calculator"), "Calculator");
        assert_eq!(clean_title("  Calculator  "), "Calculator");
    }

    #[test]
    fn allowlisted_title_matches_are_case_and_substring_insensitive() {
        assert!(is_allowlisted_title("Task Manager"));
        assert!(is_allowlisted_title("task manager"));
        assert!(is_allowlisted_title("Windows Security - Virus & threat protection"));
        assert!(!is_allowlisted_title("Notepad"));
    }

    #[test]
    fn blocklisted_exe_matches_are_case_insensitive_but_exact() {
        assert!(is_blocklisted_exe("explorer.exe"));
        assert!(is_blocklisted_exe("EXPLORER.EXE"));
        // Exact match only — a substring match here would risk blocking
        // unrelated executables that merely contain "explorer" in their name.
        assert!(!is_blocklisted_exe("myexplorer.exe"));
        assert!(!is_blocklisted_exe("notepad.exe"));
    }

    fn fake_handle(pid: u32, title: &str) -> Handle {
        Handle {
            hwnd: HWND(pid as isize),
            pid,
            start_time: 0,
            title: title.to_string(),
        }
    }

    #[test]
    fn drain_handles_empties_the_store_and_returns_every_entry_in_order() {
        let handles = Handles(Mutex::new(vec![
            fake_handle(111, "Notepad"),
            fake_handle(222, "Calculator"),
        ]));

        let drained = drain_handles(&handles).expect("store is not poisoned");

        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].pid, 111);
        assert_eq!(drained[1].pid, 222);
        assert!(
            handles.0.lock().unwrap().is_empty(),
            "drain must leave the store empty"
        );
    }

    #[test]
    fn drain_handles_on_an_empty_store_returns_empty() {
        let handles = Handles(Mutex::new(Vec::new()));
        let drained = drain_handles(&handles).expect("store is not poisoned");
        assert!(drained.is_empty());
    }

    #[test]
    fn a_second_drain_after_the_first_never_resees_old_entries() {
        // Guards the exact invariant `restore_all`'s doc comment relies on: a
        // second stash/restore cycle can't resume a stale, since-recycled PID
        // because the store was emptied by the first drain.
        let handles = Handles(Mutex::new(vec![fake_handle(111, "Notepad")]));

        let first = drain_handles(&handles).unwrap();
        let second = drain_handles(&handles).unwrap();

        assert_eq!(first.len(), 1);
        assert!(second.is_empty());
    }

    #[test]
    fn persisted_handle_round_trips_through_json() {
        let original = PersistedHandle {
            pid: 4242,
            hwnd: -17, // HWND values are signed; a real one could be negative.
            start_time: 133_500_000_000_000_000,
            title: "Notepad".to_string(),
        };

        let json = serde_json::to_string(&vec![original.clone()]).unwrap();
        let restored: Vec<PersistedHandle> = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].pid, original.pid);
        assert_eq!(restored[0].hwnd, original.hwnd);
        assert_eq!(restored[0].start_time, original.start_time);
        assert_eq!(restored[0].title, original.title);
    }

    #[test]
    fn records_written_before_start_time_tracking_still_deserialize() {
        // #[serde(default)] keeps a record written by an older build readable
        // after an upgrade; without it, every field-add would look like file
        // corruption and strand a real stashed session.
        let legacy = r#"[{"pid":4242,"hwnd":-17,"title":"Notepad"}]"#;

        let restored: Vec<PersistedHandle> =
            serde_json::from_str(legacy).expect("legacy records must still parse");

        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].start_time, 0);
    }

    #[test]
    fn a_zero_start_time_falls_back_to_plain_liveness() {
        // Our own process is certainly alive, so a legacy record (start_time
        // 0) must be treated as "same process", while a mismatched non-zero
        // timestamp must not be.
        let own_pid = unsafe { GetCurrentProcessId() };

        assert!(is_same_process(own_pid, 0));
        assert!(!is_same_process(own_pid, 1));
    }

    #[test]
    fn a_matching_start_time_identifies_the_same_process() {
        let own_pid = unsafe { GetCurrentProcessId() };
        let start = process_start_time(own_pid).expect("our own start time is readable");

        assert!(start > 0);
        assert!(is_same_process(own_pid, start));
    }
}

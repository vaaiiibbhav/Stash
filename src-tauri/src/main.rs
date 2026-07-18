// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::{engine::general_purpose, Engine as _};
use screenshots::Screen;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

use tauri::{
    AppHandle, CustomMenuItem, Manager, State, SystemTray, SystemTrayEvent, SystemTrayMenu,
};

use windows::core::{s, PCSTR, PWSTR};
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM, NTSTATUS};
use windows::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
use windows::Win32::System::Threading::{
    GetCurrentProcessId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SUSPEND_RESUME,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowInfo, GetWindowTextW, GetWindowThreadProcessId, ShowWindowAsync, SW_HIDE,
    SW_SHOW, WINDOWINFO, WS_VISIBLE,
};

/// A window we stashed: the window to re-show and the process to resume.
/// Both are always known at capture time, so `pid` is not optional. `title`
/// is carried along purely for crash recovery (see `PersistedHandle`) — nothing
/// in the live suspend/resume path reads it.
#[derive(Clone)]
struct Handle {
    hwnd: HWND,
    pid: u32,
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
    title: String,
}

fn stash_file_path(app: &AppHandle) -> Option<PathBuf> {
    let dir = app.path_resolver().app_local_data_dir()?;
    fs::create_dir_all(&dir).ok()?;
    Some(dir.join("stashed-session.json"))
}

/// Best-effort: a failure to persist must never block or fail the stash
/// itself (the windows are already suspended and hidden by the time this is
/// called), so errors are logged and swallowed.
fn persist_handles(app: &AppHandle, handles: &[Handle]) {
    let Some(path) = stash_file_path(app) else {
        eprintln!("persist_handles: no app-local-data dir available");
        return;
    };
    let persisted: Vec<PersistedHandle> = handles
        .iter()
        .map(|h| PersistedHandle {
            pid: h.pid,
            hwnd: h.hwnd.0,
            title: h.title.clone(),
        })
        .collect();
    match serde_json::to_string(&persisted) {
        Ok(json) => {
            if let Err(e) = fs::write(&path, json) {
                eprintln!("persist_handles: failed to write {path:?}: {e}");
            }
        }
        Err(e) => eprintln!("persist_handles: failed to serialize: {e}"),
    }
}

fn clear_persisted(app: &AppHandle) {
    if let Some(path) = stash_file_path(app) {
        // NotFound is the expected steady state (nothing stashed); anything
        // else is worth knowing about.
        if let Err(e) = fs::remove_file(&path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                eprintln!("clear_persisted: failed to remove {path:?}: {e}");
            }
        }
    }
}

fn read_persisted(app: &AppHandle) -> Vec<PersistedHandle> {
    let Some(path) = stash_file_path(app) else {
        return Vec::new();
    };
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
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

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Safe round-trip probe used by the frontend to confirm IPC works.
#[tauri::command]
fn ping(message: String) -> Result<String, String> {
    Ok(format!("pong from Rust: {message}"))
}

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

        // Suspend first; only hide the window once suspension has actually
        // succeeded, so we never leave a window hidden while its process runs.
        match set_process_suspended(pid, true) {
            Ok(()) => {
                let _ = unsafe { ShowWindowAsync(entry.hwnd, SW_HIDE) };
                store.push(Handle {
                    hwnd: entry.hwnd,
                    pid,
                    title: clean_title(&entry.title),
                });
                suspended += 1;
                if preview.len() < 3 {
                    preview.push(clean_title(&entry.title));
                }
            }
            // Access-denied on elevated processes is expected: skip and carry on.
            Err(reason) => eprintln!("skipping window (pid {pid}): {reason}"),
        }
    }

    // Mirror the just-built store to disk so a crash/force-kill before the
    // next successful restore can still be recovered from on next launch.
    persist_handles(&app, store.as_slice());

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
        Ok(())
    } else {
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
/// elsewhere. `count` only reflects entries whose process is still alive;
/// entries for processes that already exited are dropped from the record.
#[tauri::command]
fn check_orphaned_stash(app: AppHandle) -> Result<(bool, String, usize), String> {
    let persisted = read_persisted(&app);
    if persisted.is_empty() {
        return Ok((false, String::new(), 0));
    }

    let alive: Vec<&PersistedHandle> = persisted
        .iter()
        .filter(|h| process_exe_name(h.pid).is_some())
        .collect();

    if alive.is_empty() {
        // Every recorded process is already gone (closed manually, or reaped
        // by the OS); the stale record no longer describes anything real.
        clear_persisted(&app);
        return Ok((false, String::new(), 0));
    }

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
        if process_exe_name(entry.pid).is_none() {
            continue; // already gone; nothing to resume
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
        Ok(())
    } else {
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

fn main() {
    let quit = CustomMenuItem::new("quit".to_string(), "Quit");
    let show = CustomMenuItem::new("show".to_string(), "Show");
    let tray_menu = SystemTrayMenu::new().add_item(show).add_item(quit);
    let tray = SystemTray::new().with_menu(tray_menu);

    tauri::Builder::default()
        .setup(|app| {
            let main_window = app.get_window("main").unwrap();
            main_window.set_always_on_top(true).expect("Oopsie");
            Ok(())
        })
        .on_window_event(|event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event.event() {
                event.window().hide().unwrap();
                api.prevent_close();
            }
        })
        .system_tray(tray)
        .on_system_tray_event(|app, event| match event {
            SystemTrayEvent::LeftClick { .. } => {
                let window = app.get_window("main").unwrap();
                window.show().unwrap();
                window.set_focus().unwrap();
            }
            SystemTrayEvent::MenuItemClick { id, .. } => match id.as_str() {
                "quit" => {
                    // Resume everything we stashed before exiting, natively —
                    // no fragile __TAURI_INVOKE__ eval, and this actually exits.
                    if let Some(handles) = app.try_state::<Handles>() {
                        if let Err(reason) = restore_all(app, handles.inner()) {
                            eprintln!("restore-on-quit failed: {reason}");
                        }
                    }
                    app.exit(0);
                }
                "show" => {
                    let window = app.get_window("main").unwrap();
                    window.show().unwrap();
                }
                _ => {}
            },
            _ => {}
        })
        .manage(Handles(Default::default()))
        .invoke_handler(tauri::generate_handler![
            get_apps,
            restore,
            screen_shot,
            ping,
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
            title: "Notepad".to_string(),
        };

        let json = serde_json::to_string(&vec![original.clone()]).unwrap();
        let restored: Vec<PersistedHandle> = serde_json::from_str(&json).unwrap();

        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].pid, original.pid);
        assert_eq!(restored[0].hwnd, original.hwnd);
        assert_eq!(restored[0].title, original.title);
    }
}

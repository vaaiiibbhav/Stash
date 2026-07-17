// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use base64::{engine::general_purpose, Engine as _};
use screenshots::Screen;
use std::sync::Mutex;

use tauri::{CustomMenuItem, Manager, State, SystemTray, SystemTrayEvent, SystemTrayMenu};

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
/// Both are always known at capture time, so `pid` is not optional.
#[derive(Clone, Copy)]
struct Handle {
    hwnd: HWND,
    pid: u32,
}

/// Shared store of stashed windows, guarded by a mutex.
struct Handles(Mutex<Vec<Handle>>);

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

/// Enumerates visible top-level windows, suspends their owning processes, and
/// hides them. Returns a short comma-separated preview of up to three window
/// titles and the number of processes actually suspended.
#[tauri::command]
fn get_apps(handles: State<Handles>) -> Result<(String, usize), String> {
    // System surfaces we never stash, matched loosely by window title. Our own
    // window is excluded by PID (below), so it is deliberately absent here.
    const TITLE_ALLOWLIST: [&str; 2] = ["Task Manager", "Windows Security"];
    // Suspending these would freeze the Windows shell (taskbar, desktop, tray),
    // including our own restore-from-tray path. Matched by executable name.
    const EXE_BLOCKLIST: [&str; 1] = ["explorer.exe"];

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

        if TITLE_ALLOWLIST
            .iter()
            .any(|allowed| entry.title.to_lowercase().contains(&allowed.to_lowercase()))
        {
            continue;
        }

        if let Some(exe) = process_exe_name(pid) {
            if EXE_BLOCKLIST.iter().any(|blocked| exe.eq_ignore_ascii_case(blocked)) {
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

    Ok((preview.join(", "), suspended))
}

/// Resumes and re-shows every stashed window. Exposed as a command; the tray
/// "Quit" path calls the shared implementation directly.
#[tauri::command]
fn restore(handles: State<Handles>) -> Result<(), String> {
    restore_all(handles.inner())
}

/// Drains the window store and resumes/re-shows each entry. Draining (rather
/// than cloning) guarantees a second stash/restore cycle can never resume a
/// stale, since-recycled PID or re-show a window we no longer own.
fn restore_all(handles: &Handles) -> Result<(), String> {
    let drained: Vec<Handle> = {
        let mut store = handles
            .0
            .lock()
            .map_err(|_| "window store is poisoned".to_string())?;
        std::mem::take(&mut *store)
    };

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
                        if let Err(reason) = restore_all(handles.inner()) {
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
        .invoke_handler(tauri::generate_handler![get_apps, restore, screen_shot, ping])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

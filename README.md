# Stash 🪄

Too many windows open? Meeting in two minutes?

**Stash them.** One click suspends and hides every open app; one click brings
them all back exactly where they were.

Because Stash *suspends* the processes rather than just minimising them, the
stashed apps stop using CPU entirely — which is the difference between hiding
a video call and actually getting your battery back.

## Features

- Suspend and hide every open app in one click, and restore them just as fast
- Real process suspension (`NtSuspendProcess`), so stashed apps use no CPU
- Optional auto-restore timer — 15 minutes, 40 minutes, or an hour
- Lives in the system tray; restores everything on quit
- Crash recovery: if Stash is killed while apps are stashed, the next launch
  offers to resume them
- Accessible light and dark themes, following your OS preference

Windows only. Stash suspends processes and manipulates windows through the
Win32 and ntdll APIs, which have no cross-platform equivalent.

## Usage

1. Pick an auto-restore delay (or leave it on *None*).
2. Click **Stash your apps** — your windows disappear and their processes
   freeze. Stash stays pinned on top so you can always get back.
3. Click **Restore** to bring everything back, or wait for the timer.

Closing the window hides it to the tray rather than quitting, so a stash is
never lost by accident. **Quit** from the tray menu restores everything first.

System-critical windows are never stashed: Task Manager and Windows Security
are skipped by title, Windows Explorer by executable name (suspending it would
freeze the taskbar and desktop), and Stash skips its own process by PID.

## Prerequisites

| Requirement | Version | Notes |
| --- | --- | --- |
| [Rust](https://rustup.rs/) | stable | Backend |
| [.NET SDK](https://dotnet.microsoft.com/download) | 8.0+ | Compiles the F# frontend |
| [Node.js](https://nodejs.org/) | 20+ | Vite build pipeline |
| MSVC Build Tools | 2022 | "Desktop development with C++" — provides the linker |
| WebView2 Runtime | — | Preinstalled on Windows 11 |

The MSVC C++ workload is required: without it the Rust build fails at link
time with `linker 'link.exe' not found`.

```powershell
winget install Microsoft.VisualStudio.2022.BuildTools
```

## Development

```bash
npm install          # once
npm run tauri dev    # run the app with hot reload
```

`npm run tauri dev` starts Vite (which compiles F# → JavaScript through
`vite-plugin-fable`) and the Rust backend together.

### Useful commands

| Command | What it does |
| --- | --- |
| `npm run build` | Production frontend build (F# → JS → `dist/`) |
| `npm run check` | Type-check the F# project alone |
| `npm run tauri build` | Build the release binary and installers |
| `dotnet test tests/Stash.Tests/Stash.Tests.fsproj` | F# unit tests |
| `cargo test --manifest-path src-tauri/Cargo.toml` | Rust unit tests |
| `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets` | Rust lints |

## Architecture

```
src/                  F# frontend (Fable → React via Feliz)
  Types.fs              Domain types + the session state machine
  Tauri.fs              Typed bindings for the Rust commands
  MainUI.fs             Components and theming
  App.fs                Entry point
src-tauri/            Rust backend (Tauri 2)
  src/main.rs           Commands, Win32 suspension, tray, crash recovery
  capabilities/         Tauri ACL — what the frontend may call
tests/Stash.Tests/    F# unit tests (xUnit)
```

The frontend never touches the window or process APIs directly. Its only
contact with the backend is `invoke` on five commands — `get_apps`, `restore`,
`screen_shot`, `check_orphaned_stash`, `resume_orphaned` — each returning a
`Result` that the F# side maps into its own `Result` type.

### Where Stash keeps its files

| File | Location |
| --- | --- |
| Crash-recovery record | `%LOCALAPPDATA%\com.vaibhavverma.stash\stashed-session.json` |
| Logs | `%LOCALAPPDATA%\com.vaibhavverma.stash\logs\stash.log` |

The recovery record is written atomically and is cleared on every clean
restore, so its presence at startup means the previous run ended abnormally.

## Releasing

Push a version tag and CI builds the installers and drafts a GitHub Release:

```bash
git tag v0.2.0 && git push origin v0.2.0
```

The version comes from `src-tauri/Cargo.toml` — `tauri.conf.json` deliberately
omits `version` so there is a single source of truth. Keep `package.json` in
sync and make the tag match.

## Contributing

See [Contributing.md](Contributing.md).

## License

MIT — see [LICENSE](LICENSE).

Stash began as a fork of [Defer](https://github.com/Om-Thorat/Defer) by Om
Thorat, whose copyright is retained alongside ours. The frontend has since
been rewritten in F#/Fable, the backend moved from a bundled Sysinternals
sidecar to direct ntdll calls, and the app migrated to Tauri 2.

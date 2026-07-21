# Contributing to Stash

Thank you for considering contributing to Stash!

## Development platform

Stash is a desktop tool for Windows, so Windows is the appropriate development
platform. Process suspension goes through `NtSuspendProcess`/`NtResumeProcess`
and window management through Win32 — neither has a cross-platform equivalent,
so the backend simply cannot run elsewhere.

The frontend is plain F#/Fable and compiles on any platform, so UI work can be
done anywhere; you just can't exercise the stash/restore flow off Windows.

## File structure

* `src/` — the F# frontend, compiled to JavaScript by Fable and rendered with
  React through Feliz. `Types.fs` holds the domain types and session state
  machine, `Tauri.fs` the typed bindings to the Rust commands, `MainUI.fs` the
  components, and `App.fs` the entry point.
* `src-tauri/` — the Rust backend. `src/main.rs` houses the Tauri commands,
  the Win32 suspension logic, the tray, and crash recovery.
* `src-tauri/capabilities/` — the Tauri ACL, declaring what the frontend is
  permitted to call.
* `tests/Stash.Tests/` — F# unit tests (xUnit). Rust tests live inline in
  `main.rs` under `#[cfg(test)]`.

Process suspension is done natively from Rust via `NtSuspendProcess`/
`NtResumeProcess` (the `windows` crate) — there is no external sidecar binary
to manage.

## Requirements

Stash is a Tauri 2 app: Rust for the backend, F#/Fable rendered in a WebView
for the frontend.

1. Rust and Cargo (stable).
2. The .NET SDK 8.0+ — Fable needs it to compile the F# project.
3. Node and npm (20+).
4. MSVC Build Tools 2022 with the "Desktop development with C++" workload,
   which provides the linker Rust needs on Windows.

See the [README](README.md#prerequisites) for install commands.

## Getting started

1. Fork and clone the repo.
2. Install all the dependencies with `npm install`.
3. Run the project with `npm run tauri dev`.

Tauri docs for further reference are at [v2.tauri.app](https://v2.tauri.app/).

## Before you open a PR

Please make sure these all pass:

```bash
dotnet test tests/Stash.Tests/Stash.Tests.fsproj
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
npm run build
```

Two conventions worth knowing:

* **Every Tauri command returns `Result<T, String>`**, and the F# side maps it
  into an F# `Result`. Please don't add a command that can panic on a path the
  user can reach — a panic while apps are suspended strands them.
* **Colour choices are checked against WCAG AA.** If you add a token to
  `MainUI.fs`, include the computed contrast ratio in a comment the way the
  existing ones do.

## Common gotchas

Stash runs a little slower in dev mode, largely because the screenshot capture
is unoptimised in debug builds. Rust and Tauri apply their optimisations when
compiling for release, so be a little patient.

If the Rust build fails with `linker 'link.exe' not found`, you're missing the
MSVC C++ workload from the requirements above.

## Reporting issues

If you encounter any bugs, issues, or have suggestions for improvements, please
open an issue on the GitHub repository. When reporting an issue, please provide
as much detail as possible, including steps to reproduce the problem and any
relevant error messages. Attaching `%LOCALAPPDATA%\com.vaibhavverma.stash\logs\stash.log`
helps a great deal.

## Contact

If you have any questions or need further assistance, please open an issue or
discussion on the GitHub repository.

Happy contributing!

<h3 align="center">Stash began as a fork of <a href="https://github.com/Om-Thorat/Defer">Defer</a> by Om 💖</h3>

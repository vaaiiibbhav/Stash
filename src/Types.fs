module Stash.Types

/// UI color scheme. The app follows the OS preference on launch
/// and lets the user toggle at runtime.
[<RequireQualifiedAccess>]
type Theme =
    | Light
    | Dark

/// State of the frontend -> Rust backend IPC round-trip probe.
[<RequireQualifiedAccess>]
type BackendStatus =
    | Idle
    | Checking
    | Connected of reply: string
    | Failed of error: string

module Stash.Types

/// UI color scheme. The app follows the OS preference on launch
/// and lets the user toggle at runtime.
[<RequireQualifiedAccess>]
type Theme =
    | Light
    | Dark

/// A user-selectable delay after which a stash is auto-restored.
[<RequireQualifiedAccess>]
type AutoRestoreDelay =
    | Off
    | Minutes15
    | Minutes40
    | Hour1

module AutoRestoreDelay =
    /// Stable identifier for the <option> value — kept distinct from the
    /// display label so relabeling can never break the selected-value match.
    let key =
        function
        | AutoRestoreDelay.Off -> "off"
        | AutoRestoreDelay.Minutes15 -> "15m"
        | AutoRestoreDelay.Minutes40 -> "40m"
        | AutoRestoreDelay.Hour1 -> "1h"

    let label =
        function
        | AutoRestoreDelay.Off -> "None"
        | AutoRestoreDelay.Minutes15 -> "15 min"
        | AutoRestoreDelay.Minutes40 -> "40 min"
        | AutoRestoreDelay.Hour1 -> "1 hr"

    let seconds =
        function
        | AutoRestoreDelay.Off -> None
        | AutoRestoreDelay.Minutes15 -> Some 900
        | AutoRestoreDelay.Minutes40 -> Some 2400
        | AutoRestoreDelay.Hour1 -> Some 3600

    let all =
        [ AutoRestoreDelay.Off
          AutoRestoreDelay.Minutes15
          AutoRestoreDelay.Minutes40
          AutoRestoreDelay.Hour1 ]

/// Result of a `get_apps` call: a short preview of what was found and how
/// many processes were actually suspended.
type StashSummary = { Preview: string; Count: int }

/// A completed stash: the summary plus a screenshot of the desktop as it
/// looked right before its apps were hidden, and — if auto-restore is on —
/// the wall-clock deadline to resume automatically.
type StashSnapshot =
    { Preview: string
      Count: int
      ScreenshotBase64: string
      AutoRestoreDeadlineMs: float option }

/// The stash lifecycle. Transitions are strictly linear:
/// Idle -> Stashing -> Stashed -> Restoring -> Idle.
[<RequireQualifiedAccess>]
type SessionState =
    | Idle
    | Stashing
    | Stashed of StashSnapshot
    | Restoring

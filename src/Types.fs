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

/// Pure formatting for the auto-restore countdown. Kept free of any
/// React/Feliz dependency (unlike the rest of the UI layer) so it can be unit
/// tested directly from a plain .NET test project.
module Countdown =
    /// Precise mm:ss for sighted users. Never called with a negative value —
    /// callers clamp `secondsRemaining` to >= 0 first.
    let formatVisual (secondsRemaining: int) =
        let minutes = secondsRemaining / 60
        let seconds = secondsRemaining % 60
        $"{minutes}m {seconds}s"

    /// A coarser, minute-granular phrasing for the screen-reader
    /// announcement. Rendering this — instead of the second-by-second
    /// `formatVisual` — inside an aria-live region matters because React only
    /// touches the DOM, and so only triggers a live-region announcement, when
    /// the rendered text actually changes. Announcing every second for up to
    /// an hour would bury assistive-tech users in a stream of updates; this
    /// text stays identical for 59 out of every 60 renders, so it only
    /// announces on the minute.
    let formatAnnouncement (secondsRemaining: int) =
        let minutes = secondsRemaining / 60
        if minutes <= 0 then
            "Auto restore in under a minute"
        else
            let unit = if minutes = 1 then "minute" else "minutes"
            $"Auto restore in about {minutes} {unit}"

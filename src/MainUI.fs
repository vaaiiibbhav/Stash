module Stash.MainUI

open Browser.Dom
open Fable.Core
open Fable.Core.JsInterop
open Feliz
open Stash.Types

/// Tailwind class tokens per theme. Every text/background pair below meets
/// WCAG 2.1 AA (>= 4.5:1 for body text, >= 3:1 for large text / UI components),
/// computed by hand against the sRGB relative-luminance formula.
type private Tokens =
    { Surface: string
      Heading: string
      Body: string
      Muted: string
      Card: string
      AccentButton: string
      ToggleButton: string
      /// Count badge: white text on rose-600 — 4.70:1 (AA), and rose-600
      /// against either card background clears the 3:1 non-text minimum.
      Badge: string
      /// Error banner: computed 11.84:1 (dark) / 9.60:1 (light).
      DangerBanner: string
      SelectField: string
      Spinner: string }

let private tokens theme =
    match theme with
    | Theme.Dark ->
        // e.g. body #C7C8DE on #0B0A1A ~ 11.9:1
        { Surface = "bg-[#0B0A1A] text-[#C7C8DE]"
          Heading = "text-[#B0AFD7]"
          Body = "text-[#C7C8DE]"
          Muted = "text-[#9EA0BF]"
          Card = "bg-[#16152E] border border-[#2E2D52]"
          AccentButton =
            "bg-[#BEC0E2] text-[#14132B] hover:bg-[#D3D5F0] focus-visible:outline focus-visible:outline-2 focus-visible:outline-[#BEC0E2]"
          ToggleButton =
            "border border-[#4A4980] text-[#C7C8DE] hover:bg-[#1D1C3A] focus-visible:outline focus-visible:outline-2 focus-visible:outline-[#BEC0E2]"
          Badge = "bg-[#E11D48] text-white"
          DangerBanner = "bg-[#2A0E16] text-[#FFC2D1]"
          SelectField = "bg-[#1D1C3A] text-[#C7C8DE] border border-[#4A4980]"
          Spinner = "border-[#BEC0E2]" }
    | Theme.Light ->
        // e.g. body #3A3A55 on #F6F6FB ~ 9.4:1
        { Surface = "bg-[#F6F6FB] text-[#3A3A55]"
          Heading = "text-[#232252]"
          Body = "text-[#3A3A55]"
          Muted = "text-[#4F5070]"
          Card = "bg-white border border-[#D8D8EA]"
          AccentButton =
            "bg-[#3B3A8F] text-white hover:bg-[#4B4AAB] focus-visible:outline focus-visible:outline-2 focus-visible:outline-[#3B3A8F]"
          ToggleButton =
            "border border-[#3B3A8F] text-[#232252] hover:bg-[#E8E8F5] focus-visible:outline focus-visible:outline-2 focus-visible:outline-[#3B3A8F]"
          Badge = "bg-[#E11D48] text-white"
          DangerBanner = "bg-[#FDECEF] text-[#7A0E27]"
          SelectField = "bg-white text-[#232252] border border-[#3B3A8F]"
          Spinner = "border-[#3B3A8F]" }

let private initialTheme () =
    // matchMedia is missing from Fable.Browser.Dom's Window binding; go dynamic.
    let prefersLight: bool = !!window?matchMedia("(prefers-color-scheme: light)")?matches
    if prefersLight then Theme.Light else Theme.Dark

[<Emit("Date.now()")>]
let private nowMs () : float = jsNative

[<ReactComponent>]
let private ThemeToggle (theme: Theme) (t: Tokens) (onToggle: unit -> unit) =
    let isDark = theme = Theme.Dark

    Html.button
        [ prop.type' "button"
          prop.className $"rounded px-3 py-1.5 text-sm font-semibold transition-colors {t.ToggleButton}"
          prop.ariaPressed isDark
          prop.ariaLabel "Toggle dark mode"
          prop.onClick (fun _ -> onToggle ())
          prop.text (
              match theme with
              | Theme.Dark -> "Switch to light"
              | Theme.Light -> "Switch to dark"
          ) ]

[<ReactComponent>]
let private ErrorBanner (t: Tokens) (message: string) (onDismiss: unit -> unit) =
    Html.div
        [ prop.role "alert"
          prop.className $"w-full max-w-md rounded-lg px-4 py-3 flex items-start justify-between gap-3 text-sm {t.DangerBanner}"
          prop.children
              [ Html.span [ prop.key "message"; prop.className "flex-1"; prop.text message ]
                Html.button
                    [ prop.key "dismiss"
                      prop.type' "button"
                      prop.ariaLabel "Dismiss error"
                      prop.className
                          "font-bold leading-none opacity-70 hover:opacity-100 rounded focus-visible:outline focus-visible:outline-2 focus-visible:outline-current"
                      prop.onClick (fun _ -> onDismiss ())
                      prop.text "×" ] ] ]

/// Shown when `checkOrphanedStash` finds a stash record left behind by a
/// previous run that didn't exit cleanly (crash, force-kill, logoff — the
/// normal Quit and Restore paths both clear this record). Non-interrupting
/// (role="status", not "alert"): the affected apps are already suspended and
/// safe, so this is informational, not urgent.
[<ReactComponent>]
let private OrphanBanner (t: Tokens) (summary: StashSummary) (onResume: unit -> unit) (onDismiss: unit -> unit) =
    let description =
        let apps = if summary.Count = 1 then "1 app" else $"{summary.Count} apps"
        $"Stash didn't close cleanly last time — {apps} may still be stashed: {summary.Preview}."

    Html.div
        [ prop.role "status"
          prop.ariaLive.polite
          prop.className $"w-full max-w-md rounded-lg p-4 flex flex-col gap-3 text-sm {t.Card} {t.Body}"
          prop.children
              [ Html.p [ prop.key "message"; prop.text description ]
                Html.div
                    [ prop.key "actions"
                      prop.className "flex gap-2 justify-end"
                      prop.children
                          [ Html.button
                                [ prop.key "dismiss"
                                  prop.type' "button"
                                  prop.className
                                      $"rounded px-3 py-1.5 text-sm font-semibold transition-colors {t.ToggleButton}"
                                  prop.onClick (fun _ -> onDismiss ())
                                  prop.text "Dismiss" ]
                            Html.button
                                [ prop.key "resume"
                                  prop.type' "button"
                                  prop.className
                                      $"rounded px-3 py-1.5 text-sm font-bold transition-colors {t.AccentButton}"
                                  prop.onClick (fun _ -> onResume ())
                                  prop.text "Resume now" ] ] ] ] ]

[<ReactComponent>]
let private DelaySelect (t: Tokens) (value: AutoRestoreDelay) (onChange: AutoRestoreDelay -> unit) =
    Html.label
        [ prop.className $"flex items-center gap-2 text-sm {t.Muted}"
          prop.children
              [ Html.span [ prop.key "label"; prop.text "Auto restore in" ]
                Html.select
                    [ prop.key "select"
                      prop.ariaLabel "Auto restore delay"
                      prop.className $"rounded px-2 py-1 text-sm {t.SelectField}"
                      prop.value (AutoRestoreDelay.key value)
                      prop.onChange (fun (raw: string) ->
                          AutoRestoreDelay.all
                          |> List.tryFind (fun d -> AutoRestoreDelay.key d = raw)
                          |> Option.iter onChange)
                      prop.children
                          [ for d in AutoRestoreDelay.all ->
                                Html.option
                                    [ prop.key (AutoRestoreDelay.key d)
                                      prop.value (AutoRestoreDelay.key d)
                                      prop.text (AutoRestoreDelay.label d) ] ] ] ] ]

[<ReactComponent>]
let private IdleView
    (t: Tokens)
    (delay: AutoRestoreDelay)
    (onDelayChange: AutoRestoreDelay -> unit)
    (onStash: unit -> unit)
    =
    Html.div
        [ prop.className "w-full max-w-md flex flex-col items-center gap-4"
          prop.children
              [ Html.button
                    [ prop.key "stash-button"
                      prop.type' "button"
                      prop.className $"w-full rounded px-4 py-2.5 font-bold tracking-wide transition-colors {t.AccentButton}"
                      prop.onClick (fun _ -> onStash ())
                      prop.text "Stash your apps 👋" ]
                DelaySelect t delay onDelayChange ] ]

[<ReactComponent>]
let private BusyView (t: Tokens) (label: string) =
    Html.div
        [ prop.role "status"
          prop.ariaLive.polite
          prop.className $"w-full max-w-md flex flex-col items-center gap-3 py-6 {t.Muted}"
          prop.children
              [ Html.div
                    [ prop.key "spinner"
                      prop.className $"h-8 w-8 rounded-full border-2 border-t-transparent animate-spin {t.Spinner}"
                      prop.ariaHidden true ]
                Html.span [ prop.key "label"; prop.text label ] ] ]

[<ReactComponent>]
let private StashedView (t: Tokens) (snapshot: StashSnapshot) (onRestore: unit -> unit) =
    // A per-second tick that forces a re-render; the remaining time is always
    // recomputed from the wall-clock deadline (not decremented), so it can't
    // drift if the tab is throttled in the background. The tick value is the
    // timestamp itself (not a counter) so the interval callback never needs
    // to read a previous tick from its closure — Feliz's useState setter only
    // accepts a plain value, not a `'T -> 'T` updater, so a counter would
    // otherwise require closing over a stale `tick` from setup time.
    let tick, setTick = React.useState 0.0

    React.useEffect (
        (fun () ->
            match snapshot.AutoRestoreDeadlineMs with
            | None -> React.createDisposable (fun () -> ())
            | Some _ ->
                // setInterval/clearInterval live on Window (Browser.Types.WindowTimers),
                // not Fable.Core.JS; the trailing obj[] mirrors the DOM API's
                // optional extra handler args, which we don't use.
                let intervalId = window.setInterval ((fun () -> setTick (nowMs ())), 1000, [||])
                React.createDisposable (fun () -> window.clearInterval intervalId)),
        [| box snapshot.AutoRestoreDeadlineMs |]
    )

    let remainingSeconds =
        snapshot.AutoRestoreDeadlineMs
        |> Option.map (fun deadline -> max 0 (int ((deadline - nowMs ()) / 1000.0)))

    React.useEffect (
        (fun () ->
            match remainingSeconds with
            | Some 0 -> onRestore ()
            | _ -> ()),
        [| box tick |]
    )

    Html.section
        [ prop.className $"w-full max-w-md rounded-lg p-5 flex flex-col gap-4 {t.Card}"
          prop.children
              [ Html.div
                    [ prop.key "screenshot-wrap"
                      prop.className "relative"
                      prop.children
                          [ (if System.String.IsNullOrEmpty snapshot.ScreenshotBase64 then
                                 // Screenshot capture failed (see the error banner); an
                                 // empty src would render the browser's broken-image icon.
                                 Html.div
                                     [ prop.key "screenshot"
                                       prop.className
                                           $"w-full aspect-video rounded border border-black/10 flex items-center justify-center text-xs {t.Muted}"
                                       prop.text "No preview available" ]
                             else
                                 Html.img
                                     [ prop.key "screenshot"
                                       prop.className "w-full rounded border border-black/10 object-cover"
                                       prop.src $"data:image/png;base64,{snapshot.ScreenshotBase64}"
                                       prop.alt "Your desktop just before its apps were stashed" ])
                            Html.span
                                [ prop.key "badge"
                                  prop.className
                                      $"absolute -top-2 -right-2 min-w-6 h-6 px-1.5 rounded-full flex items-center justify-center text-xs font-bold {t.Badge}"
                                  prop.ariaHidden true
                                  prop.text (string snapshot.Count) ] ] ]
                Html.p
                    [ prop.key "preview"
                      prop.className $"text-sm text-center {t.Body}"
                      // One sentence, correct for sighted and screen-reader users alike:
                      // the count is stated in words, and "and more" only appears when
                      // Preview was actually truncated. main.rs's get_apps caps the
                      // preview at the first 3 titles it suspends, so Count > 3 is
                      // exactly the condition under which titles were left out.
                      prop.text (
                          if snapshot.Count = 0 then
                              "Nothing to stash — your desktop was already clear."
                          elif snapshot.Count > 3 then
                              $"{snapshot.Count} apps stashed: {snapshot.Preview} and more"
                          else
                              $"{snapshot.Count} apps stashed: {snapshot.Preview}"
                      ) ]
                (match remainingSeconds with
                 | Some seconds ->
                     Html.div
                         [ prop.key "countdown"
                           prop.className "text-center"
                           prop.children
                               [ Html.p
                                     [ prop.key "visual"
                                       prop.ariaHidden true
                                       prop.className $"text-xs {t.Muted}"
                                       prop.text $"Auto restore in {Countdown.formatVisual seconds}" ]
                                 Html.p
                                     [ prop.key "announcement"
                                       prop.role "status"
                                       prop.ariaLive.polite
                                       prop.className "sr-only"
                                       prop.text (Countdown.formatAnnouncement seconds) ] ] ]
                 | None -> Html.none)
                Html.button
                    [ prop.key "restore-button"
                      prop.type' "button"
                      prop.className $"rounded px-4 py-2 font-bold tracking-wide transition-colors {t.AccentButton}"
                      prop.onClick (fun _ -> onRestore ())
                      prop.text "Restore 🔁" ] ] ]

[<ReactComponent>]
let private StashPanel (t: Tokens) =
    let sessionState, setSessionState = React.useState SessionState.Idle
    let delay, setDelay = React.useState AutoRestoreDelay.Off
    let errorMessage, setErrorMessage = React.useState (None: string option)
    let orphan, setOrphan = React.useState (None: StashSummary option)

    // Runs once per app launch: ask Rust whether a previous run left a stash
    // record behind without a matching restore (crash, force-kill, logoff).
    // Best-effort — a failed check just means no recovery banner, which is
    // safe to skip rather than surface as an error on startup.
    React.useEffectOnce (fun () ->
        promise {
            match! Tauri.checkOrphanedStash () with
            | Ok found -> setOrphan found
            | Error _ -> ()
        }
        |> Promise.start)

    let stash () =
        setErrorMessage None
        setSessionState SessionState.Stashing

        promise {
            match! Tauri.getApps () with
            | Error reason ->
                setErrorMessage (Some reason)
                setSessionState SessionState.Idle
            | Ok summary ->
                // The stash already happened (windows are suspended+hidden); a
                // failed screenshot shouldn't discard that, so fall back to an
                // empty preview image rather than dropping to an error state.
                let! screenshotResult = Tauri.screenShot ()

                let screenshot, screenshotError =
                    match screenshotResult with
                    | Ok bytes -> bytes, None
                    | Error reason -> "", Some $"Stashed, but couldn't capture a preview: {reason}"

                setErrorMessage screenshotError

                setSessionState (
                    SessionState.Stashed
                        { Preview = summary.Preview
                          Count = summary.Count
                          ScreenshotBase64 = screenshot
                          AutoRestoreDeadlineMs =
                            AutoRestoreDelay.seconds delay
                            |> Option.map (fun secs -> nowMs () + float secs * 1000.0) }
                )
        }
        |> Promise.start

    let restore () =
        setErrorMessage None
        setSessionState SessionState.Restoring

        promise {
            match! Tauri.restore () with
            | Error reason -> setErrorMessage (Some reason)
            | Ok () -> ()

            // Whether or not restore fully succeeded, Idle is the only state
            // that lets the user act again (retry via Stash).
            setSessionState SessionState.Idle
        }
        |> Promise.start

    let resumeOrphan () =
        promise {
            match! Tauri.resumeOrphaned () with
            | Error reason -> setErrorMessage (Some reason)
            | Ok () -> ()

            // Rust clears the on-disk record whether or not every process
            // resumed, so there is nothing left here to re-offer either way.
            setOrphan None
        }
        |> Promise.start

    Html.div
        [ prop.className "w-full flex flex-col items-center gap-4"
          prop.children
              [ (match orphan with
                 | Some summary -> OrphanBanner t summary resumeOrphan (fun () -> setOrphan None)
                 | None -> Html.none)
                (match errorMessage with
                 | Some message -> ErrorBanner t message (fun () -> setErrorMessage None)
                 | None -> Html.none)
                (match sessionState with
                 | SessionState.Idle -> IdleView t delay setDelay stash
                 | SessionState.Stashing -> BusyView t "Stashing your apps…"
                 | SessionState.Stashed snapshot -> StashedView t snapshot restore
                 | SessionState.Restoring -> BusyView t "Restoring your apps…") ] ]

[<ReactComponent>]
let AppShell () =
    let theme, setTheme = React.useState (initialTheme ())
    let t = tokens theme

    let toggleTheme () =
        setTheme (
            match theme with
            | Theme.Dark -> Theme.Light
            | Theme.Light -> Theme.Dark
        )

    Html.div
        [ prop.className $"min-h-screen flex flex-col items-center gap-8 px-6 py-10 transition-colors {t.Surface}"
          prop.children
              [ Html.header
                    [ prop.key "header"
                      prop.className "w-full max-w-md flex items-start justify-between"
                      prop.children
                          [ Html.div
                                [ prop.key "title"
                                  prop.children
                                      [ Html.h1
                                            [ prop.key "app-name"
                                              prop.className $"text-4xl font-extrabold {t.Heading}"
                                              prop.text "Stash" ]
                                        Html.p
                                            [ prop.key "tagline"
                                              prop.className $"text-sm tracking-widest uppercase {t.Muted}"
                                              prop.text "need a break?" ] ] ]
                            ThemeToggle theme t toggleTheme ] ]
                Html.main
                    [ prop.key "main"
                      prop.className "w-full flex flex-col items-center gap-6"
                      prop.children [ StashPanel t ] ] ] ]

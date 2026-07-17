module Stash.MainUI

open Browser.Dom
open Fable.Core.JsInterop
open Feliz
open Stash.Types

/// Tailwind class tokens per theme. Every text/background pair below meets
/// WCAG 2.1 AA (>= 4.5:1 for body text, >= 3:1 for large headings).
type private Tokens =
    { Surface: string
      Heading: string
      Body: string
      Muted: string
      Card: string
      AccentButton: string
      ToggleButton: string }

let private tokens theme =
    match theme with
    | Theme.Dark ->
        // e.g. body #C7C8DE on #0B0A1A ~ 10.6:1
        { Surface = "bg-[#0B0A1A] text-[#C7C8DE]"
          Heading = "text-[#B0AFD7]"
          Body = "text-[#C7C8DE]"
          Muted = "text-[#9EA0BF]"
          Card = "bg-[#16152E] border border-[#2E2D52]"
          AccentButton =
            "bg-[#BEC0E2] text-[#14132B] hover:bg-[#D3D5F0] focus-visible:outline focus-visible:outline-2 focus-visible:outline-[#BEC0E2]"
          ToggleButton =
            "border border-[#4A4980] text-[#C7C8DE] hover:bg-[#1D1C3A] focus-visible:outline focus-visible:outline-2 focus-visible:outline-[#BEC0E2]" }
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
            "border border-[#3B3A8F] text-[#232252] hover:bg-[#E8E8F5] focus-visible:outline focus-visible:outline-2 focus-visible:outline-[#3B3A8F]" }

let private initialTheme () =
    // matchMedia is missing from Fable.Browser.Dom's Window binding; go dynamic.
    let prefersLight: bool = !!window?matchMedia("(prefers-color-scheme: light)")?matches
    if prefersLight then Theme.Light else Theme.Dark

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

/// Card that exercises the F# -> Rust IPC round trip. Owns its own
/// request state; theming comes in via tokens.
[<ReactComponent>]
let private BackendProbe (t: Tokens) =
    let status, setStatus = React.useState BackendStatus.Idle
    let isChecking = status = BackendStatus.Checking

    let probe () =
        setStatus BackendStatus.Checking

        promise {
            try
                let! reply = Tauri.ping "ping from Fable"
                setStatus (BackendStatus.Connected reply)
            with ex ->
                setStatus (BackendStatus.Failed ex.Message)
        }
        |> Promise.start

    Html.section
        [ prop.className $"w-full max-w-md rounded-lg p-5 flex flex-col gap-3 {t.Card}"
          prop.children
              [ Html.h2 [ prop.className $"text-lg font-bold {t.Heading}"; prop.text "Backend link" ]
                Html.p
                    [ prop.className $"text-sm {t.Muted}"
                      prop.text "Verifies the round trip from the Fable UI to the Rust `ping` command." ]
                Html.button
                    [ prop.type' "button"
                      prop.className
                          $"rounded px-4 py-2 font-bold tracking-wide transition-colors disabled:opacity-60 {t.AccentButton}"
                      prop.disabled isChecking
                      prop.onClick (fun _ -> probe ())
                      prop.text "Ping backend" ]
                Html.p
                    [ prop.role "status"
                      prop.ariaLive.polite
                      prop.className $"text-sm min-h-5 {t.Body}"
                      prop.text (
                          match status with
                          | BackendStatus.Idle -> "Not checked yet."
                          | BackendStatus.Checking -> "Waiting for the backend…"
                          | BackendStatus.Connected reply -> $"Connected — backend replied: “{reply}”"
                          | BackendStatus.Failed error -> $"No backend reply: {error}"
                      ) ] ] ]

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
                    [ prop.className "w-full max-w-md flex items-start justify-between"
                      prop.children
                          [ Html.div
                                [ Html.h1 [ prop.className $"text-4xl font-extrabold {t.Heading}"; prop.text "Stash" ]
                                  Html.p
                                      [ prop.className $"text-sm tracking-widest uppercase {t.Muted}"
                                        prop.text "need a break?" ] ]
                            ThemeToggle theme t toggleTheme ] ]
                Html.main [ prop.className "w-full flex flex-col items-center"; prop.children [ BackendProbe t ] ] ] ]

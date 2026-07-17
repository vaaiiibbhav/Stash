module Stash.Tauri

open Fable.Core

// Tauri 1.x exposes `invoke` from "@tauri-apps/api/tauri".
// (When the backend migrates to Tauri 2.x this import moves to "@tauri-apps/api/core".)
[<Import("invoke", from = "@tauri-apps/api/tauri")>]
let private invokeRaw<'T> (command: string) (args: obj) : JS.Promise<'T> = jsNative

/// Round-trip probe: sends a message to the Rust `ping` command and
/// resolves with the backend's reply, or rejects outside a Tauri window.
let ping (message: string) : JS.Promise<string> =
    invokeRaw<string> "ping" {| message = message |}

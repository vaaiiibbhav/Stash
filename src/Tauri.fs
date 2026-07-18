module Stash.Tauri

open Fable.Core
open Stash.Types

// Tauri 1.x exposes `invoke` from "@tauri-apps/api/tauri".
// (When the backend migrates to Tauri 2.x this import moves to "@tauri-apps/api/core".)
[<Import("invoke", from = "@tauri-apps/api/tauri")>]
let private invoke<'T> (command: string) : JS.Promise<'T> = jsNative

/// Every command here returns `Result<T, String>` on the Rust side. Tauri
/// resolves the JS promise with T on Ok, and *rejects* it with the raw Err
/// value on Err — which is a plain string for our commands, but could be a JS
/// Error if the rejection came from Tauri's own IPC layer instead (e.g. no
/// Tauri runtime present in a browser tab). Normalize both to a message
/// string rather than trusting `.Message`, which is undefined for a plain
/// string rejection.
[<Emit("($0 && $0.message) ? $0.message : String($0)")>]
let private describeRejection (reason: obj) : string = jsNative

let private attempt (work: unit -> JS.Promise<'T>) : JS.Promise<Result<'T, string>> =
    promise {
        try
            let! value = work ()
            return Ok value
        with ex ->
            return Error(describeRejection (box ex))
    }

/// Enumerates and suspends stashable windows, returning a short preview of
/// what was hidden and how many processes were actually suspended.
let getApps () : JS.Promise<Result<StashSummary, string>> =
    promise {
        // Rust returns the tuple (String, usize); Fable's runtime
        // representation of an F# tuple is a plain JS array, matching the
        // JSON Tauri sends back, so this generic invoke needs no extra parsing.
        let! result = attempt (fun () -> invoke<string * int> "get_apps")
        return result |> Result.map (fun (preview, count) -> { Preview = preview; Count = count })
    }

/// Resumes and re-shows every window stashed by the last `getApps` call.
let restore () : JS.Promise<Result<unit, string>> =
    attempt (fun () -> invoke<unit> "restore")

/// Captures the primary display and returns it as base64-encoded PNG bytes
/// (no `data:` URI prefix — callers own how it's rendered).
let screenShot () : JS.Promise<Result<string, string>> =
    attempt (fun () -> invoke<string> "screen_shot")

/// Checks for a stash left on disk by a previous run that didn't exit
/// cleanly (crash, force-kill, logoff). Rust reports this as a
/// `(found, preview, count)` tuple rather than an option-shaped object —
/// the `Some`/`None` is reconstructed here, in plain F#, rather than trusted
/// across the raw `invoke` boundary, so a malformed or absent JS `null`
/// never gets misread as a value.
let checkOrphanedStash () : JS.Promise<Result<StashSummary option, string>> =
    promise {
        let! result = attempt (fun () -> invoke<bool * string * int> "check_orphaned_stash")

        return
            result
            |> Result.map (fun (found, preview, count) ->
                if found then
                    Some { Preview = preview; Count = count }
                else
                    None)
    }

/// Resumes and re-shows every process from a leftover stash record found by
/// `checkOrphanedStash`, then clears that record.
let resumeOrphaned () : JS.Promise<Result<unit, string>> =
    attempt (fun () -> invoke<unit> "resume_orphaned")

module Stash.App

open Browser.Dom
open Fable.Core.JsInterop
open Feliz

importSideEffects "./styles.css"

let private root = ReactDOM.createRoot (document.getElementById "app")
root.render (MainUI.AppShell())

module Stash.Tests.AutoRestoreDelayTests

open Xunit
open Stash.Types

[<Fact>]
let ``every delay has a unique key`` () =
    let keys = AutoRestoreDelay.all |> List.map AutoRestoreDelay.key
    Assert.Equal<int>(keys.Length, keys |> List.distinct |> List.length)

[<Fact>]
let ``DelaySelect's lookup-by-key round-trips every case`` () =
    // Mirrors the exact pattern MainUI.fs's DelaySelect uses to turn a
    // <select> value back into a case: List.tryFind by key. If two cases
    // ever shared a key, this would silently resolve to the wrong one.
    for d in AutoRestoreDelay.all do
        let found =
            AutoRestoreDelay.all
            |> List.tryFind (fun candidate -> AutoRestoreDelay.key candidate = AutoRestoreDelay.key d)

        Assert.Equal(Some d, found)

[<Fact>]
let ``Off has no seconds; every other delay does`` () =
    Assert.Equal(None, AutoRestoreDelay.seconds AutoRestoreDelay.Off)

    for d in AutoRestoreDelay.all do
        if d <> AutoRestoreDelay.Off then
            Assert.True((AutoRestoreDelay.seconds d).IsSome, $"expected {d} to have a seconds value")

[<Fact>]
let ``seconds match their documented minute values`` () =
    Assert.Equal(Some 900, AutoRestoreDelay.seconds AutoRestoreDelay.Minutes15)
    Assert.Equal(Some 2400, AutoRestoreDelay.seconds AutoRestoreDelay.Minutes40)
    Assert.Equal(Some 3600, AutoRestoreDelay.seconds AutoRestoreDelay.Hour1)

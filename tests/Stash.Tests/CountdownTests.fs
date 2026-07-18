module Stash.Tests.CountdownTests

open Xunit
open Stash.Types

[<Theory>]
[<InlineData(0, "0m 0s")>]
[<InlineData(5, "0m 5s")>]
[<InlineData(59, "0m 59s")>]
[<InlineData(60, "1m 0s")>]
[<InlineData(61, "1m 1s")>]
[<InlineData(3599, "59m 59s")>]
[<InlineData(3600, "60m 0s")>]
let ``formatVisual renders minutes and seconds`` (secondsRemaining: int) (expected: string) =
    Assert.Equal(expected, Countdown.formatVisual secondsRemaining)

[<Theory>]
[<InlineData(0, "Auto restore in under a minute")>]
[<InlineData(30, "Auto restore in under a minute")>]
[<InlineData(59, "Auto restore in under a minute")>]
[<InlineData(60, "Auto restore in about 1 minute")>]
[<InlineData(119, "Auto restore in about 1 minute")>]
[<InlineData(120, "Auto restore in about 2 minutes")>]
[<InlineData(3600, "Auto restore in about 60 minutes")>]
let ``formatAnnouncement switches to plural only after exactly one minute`` (secondsRemaining: int) (expected: string) =
    Assert.Equal(expected, Countdown.formatAnnouncement secondsRemaining)

[<Fact>]
let ``formatAnnouncement stays constant for all 60 seconds within a minute`` () =
    // This is the load-bearing property behind the aria-live design in
    // MainUI.fs: an aria-live region only announces when its rendered text
    // actually changes, so the announcement text must NOT change every
    // second, or a screen reader would narrate it once per second for up to
    // an hour.
    let announcementsInOneMinuteWindow =
        [ 120..179 ] |> List.map Countdown.formatAnnouncement |> List.distinct

    Assert.Single(announcementsInOneMinuteWindow: string list) |> ignore

# Keep audio

Settings > Meetings has one Keep audio choice. It writes through the same Hex
settings API as the other settings, then reads both values back from Hex.

| Choice | meetings.audio_retention | labs.expire_meeting_audio | Result |
| --- | --- | --- | --- |
| 24 hours | day | true | Expire finished audio after 24 hours. |
| 7 days | week | true | Expire finished audio after 7 days. |
| 1 month | month | true | Expire finished audio after 30 days. |
| Forever | forever | false | Turn off separate audio expiry. |

Keep whole meetings is `meetings.retention`, a different setting that moves the
entire meeting to Trash. Keep it at Forever to retain text indefinitely. Choosing
Keep audio never writes that setting. A shorter whole-meeting window still removes
a meeting and its audio first. An unset audio window with the lab enabled follows
whole-meeting retention; the app reports this rather than inventing a choice.
Hex must be running to read or change Keep audio. Failed or partial writes show an
error and read back the actual settings. An unreadable reply shows no cached choice.

## What Hex expires

Hex's [Meetings retention documentation](https://github.com/anomalyco/hex/blob/main/docs/features/meetings.md#retention)
says separate expiry requires a Complete meeting with its final transcript published.
Failed, interrupted and pending recordings keep their audio. Recorder locks and
notes being written also prevent expiry. This is documented upstream behavior,
not a live expiry observation.

The [expiry source](https://github.com/anomalyco/hex/blob/main/src/meeting/retention.rs)
checks Complete status and pending/active work. It does not separately check that a
transcript file still exists or contains words. A meeting already marked Complete
with a missing or empty transcript is therefore not guaranteed to retain audio.

Hex moves `system.wav` and `microphone.wav` to Trash. It keeps transcripts, notes,
summaries and Markdown. Its pass runs at launch, hourly and soon after retention
settings change. Age comes from the meeting end time, falling back to creation time.
Nothing expires while Hex is not running. One month means 30 days in Hex.

use super::*;
use sidenote::core::meeting_parts::{self, Meeting, Phase as SavedPhase};
pub(in crate::ui) fn scene(s: &mut State, state: &str) {
    if state=="failed" { crate::ui::notes_scene::meeting(s,"helper-missing"); }
    let recording = state == "recording" || state == "transcript" || state == "resumed";
    hex::scene(s, recording);
    let id = s
        .meetings
        .first()
        .map(|m| m.id.clone())
        .unwrap_or_else(|| "1800000000000-1".into());
    let mut m = Meeting {
        id: id.clone(),
        phase: if recording {
            SavedPhase::Recording
        } else {
            SavedPhase::Paused
        },
        parts: vec![meeting_parts::Part {
            id: id.clone(),
            started_ms: 1800000000000,
            duration_ms: 724000,
        }],
        ..Default::default()
    };
    if state == "done" || state == "stale" {
        m.notes_parts = 1;
        m.requested_parts = 1;
    }
    if state == "stale" || state == "resumed" {
        m.parts.push(meeting_parts::Part {
            id: "1800001000000-2".into(),
            started_ms: 1800001000000,
            duration_ms: 62000,
        });
    }
    if state == "done" || state == "writing" || state == "failed" {
        m.phase = SavedPhase::Ended;
    }
    let mut live = Live::scene(
        if recording {
            Phase::Recording
        } else if state == "writing" || state == "failed" {
            Phase::Making
        } else {
            Phase::Paused
        },
        Some("Supplier review"),
    )
    .scene_feed(
        &[
            (0, false, "The freight quote includes the port charges."),
            (15, true, "Can we keep the old rate for two shipments?"),
            (50, false, "I will check the handling fee."),
        ],
        state == "transcript" || state == "resumed",
    );
    if state == "resumed" {
        let mut feed = live.feed.lock().unwrap_or_else(|e| e.into_inner());
        feed.lines.push(LiveLine {
            seconds: 724,
            you: false,
            text: "Recording resumed".into(),
        });
        feed.lines.push(LiveLine {
            seconds: 730,
            you: true,
            text: "The volume is confirmed for Friday.".into(),
        });
    }
    live.session = Some(m);
    s.live = Some(live);
    s.live_open = true;
    if state=="failed" { s.selected=None; }
    s.live_notes = TextEdit::new(
        "Check the handling fee on the freight quote.
Confirm volume by Friday.",
    );
    if state == "done" || state == "stale" {
        s.selected = Some(id);
        s.live_open = false;
    }
}

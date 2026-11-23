use super::*;
use crate::ui::widgets::{ButtonDisabled, dot};
use sidenote::core::{
    self,
    meeting_parts::{self, Phase as SavedPhase},
};
fn session(s: &State) -> Option<meeting_parts::Meeting> {
    s.live
        .as_ref()
        .and_then(|l| l.session.as_ref())
        .filter(|m| s.selected.as_ref().is_none_or(|id| id == &m.id))
        .cloned()
        .or_else(|| {
            s.selected
                .as_deref()
                .and_then(|id| meeting_parts::read(&core::state_dir(), id).ok().flatten())
        })
}
fn is_recording(s: &State, m: &Option<meeting_parts::Meeting>) -> bool {
    m.as_ref().is_some_and(|m| m.phase == SavedPhase::Recording)
        || (s.selected.is_none() && s.live.as_ref().is_some_and(|l| l.phase == Phase::Recording))
}
/// Level E: the line over the notes of the meeting that is recording. The notes are written when
/// it pauses, so until then they are the last ones.
pub(in crate::ui) fn recording_note(s: &State, p: Palette) -> Option<El> {
    is_recording(s, &session(s)).then(|| {
        row()
            .key("meeting-recording-note")
            .gap(8.0)
            .items_center()
            .mb(16.0)
            .child(dot(8.0, p.danger))
            .child(roles::SMALL.wrap("Recording. These notes update when you pause.", p.faint))
    })
}
pub(in crate::ui) fn parts_control(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let m = session(s);
    let recording = is_recording(s, &m);
    let open = if s.on_live() {
        s.live.as_ref().is_some_and(|l| l.panel_open)
    } else {
        s.transcript_open
    };
    let label = if open {
        "Hide transcript"
    } else {
        "Show transcript"
    };
    let transcript = pressable(
        row()
            .gap(2.0)
            .items_center()
            .justify_center()
            .child(super::super::icons::draw(
                &super::super::icons::AUDIO_LINES,
                16.0,
                p.soft,
            ))
            .child(super::super::icons::draw(
                if open {
                    &super::super::icons::CHEVRON_DOWN
                } else {
                    &super::super::icons::CHEVRON_UP
                },
                14.0,
                p.soft,
            )),
    )
    .key("meeting-transcript")
    .label(label)
    .size(52.0, 40.0)
    .radius(20.0)
    .hover(p.chip)
    .on_hover_change(|s: &mut State, hovered, _| s.meeting_ui.recording_hover = hovered)
    .on_press(|s: &mut State| {
        if s.on_live() {
            s.live_toggle_panel();
        } else {
            s.toggle_transcript();
        }
    });
    let transcript = rune_kit::Tooltip::new("meeting-recording-tip")
        .trigger(transcript)
        .text(k, label)
        .open(s.meeting_ui.recording_hover)
        .placement(if open {
            rune_kit::Placement::Bottom
        } else {
            rune_kit::Placement::Top
        })
        .build(k, layer);
    let mut control = row()
        .key("meeting-recording-control")
        .items_center()
        .padding(2.0)
        .h(44.0)
        .min_w(56.0)
        .radius(22.0)
        .fill(p.card)
        .border(1.0, p.line)
        .shadow(3.0, 16.0, p.shadow)
        .child(transcript);
    if recording {
        // Two bars distinguish Pause here from Stop on other pages.
        let bar = |key: &'static str| el().key(key).w(3.0).h(9.0).radius(1.0).fill(p.ink);
        control = control.child(
            pressable(
                row()
                    .key("meeting-pause-bars")
                    .gap(2.0)
                    .items_center()
                    .justify_center()
                    .child(bar("meeting-pause-bar-0"))
                    .child(bar("meeting-pause-bar-1")),
            )
            .key("meeting-pause")
            .label("Pause recording")
            .w(28.0)
            .h(28.0)
            .radius(14.0)
            .fill(p.bg)
            .center()
            .button_disabled(s.hex_record_busy())
            .on_press(|s: &mut State| s.meeting_pause()),
        );
    } else if m
        .as_ref()
        .is_some_and(|m| m.phase != SavedPhase::Ended || !m.parts.is_empty())
    {
        control = control.child(
            quiet_button("Resume", "meeting-resume", p)
                .button_disabled(s.hex_record_busy())
                .on_press(|s: &mut State| s.meeting_resume()),
        );
    }
    if let Some(m) = m
        && m.phase != SavedPhase::Ended
        && !m.parts.is_empty()
        && !recording
    {
        control = control.child(
            quiet_button(
                if m.stale() {
                    "Update notes"
                } else {
                    "Generate notes"
                },
                "meeting-generate",
                p,
            )
            .button_disabled(s.hex_record_busy())
            .on_press(|s: &mut State| s.meeting_generate()),
        );
    }
    control
}

pub(in crate::ui) fn status(s: &State, p: Palette) -> Option<El> {
    let m = session(s)?;
    if m.phase == SavedPhase::Ended {
        return None;
    }
    let phase = s
        .live
        .as_ref()
        .filter(|live| {
            live.session
                .as_ref()
                .is_some_and(|session| session.id == m.id)
        })
        .map(|live| live.phase)
        .unwrap_or(if m.phase == SavedPhase::Recording {
            Phase::Recording
        } else {
            Phase::Paused
        });
    // The recording line over the notes carries the notes state during recording.
    if phase == Phase::Recording {
        return None;
    }
    let state = match phase {
        Phase::Starting => "Starting",
        Phase::Recording => "Recording",
        Phase::Paused => "Paused",
        Phase::Making => "Writing notes",
    };
    let mut c = col().gap(8.0).child(
        roles::META
            .line(
                format!("{state} \u{00b7} {} recorded", stamp(m.elapsed_ms() / 1000)),
                p.soft,
            )
            .key("meeting-recorded-time"),
    );
    if m.stale() {
        c = c.child(
            roles::SMALL
                .wrap("New audio since these notes", p.soft)
                .key("meeting-notes-stale"),
        );
    }
    Some(c)
}

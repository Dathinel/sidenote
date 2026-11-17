use super::*;
use sidenote::core::{
    self,
    meeting_parts::{self, Phase as SavedPhase},
};
fn session(s: &State) -> Option<meeting_parts::Meeting> {
    s
        .live
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
pub(in crate::ui) fn parts_control(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let m = session(s);
    let recording = m.as_ref().is_some_and(|m| m.phase == SavedPhase::Recording)
        || (s.selected.is_none() && s.live.as_ref().is_some_and(|l| l.phase == Phase::Recording));
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
            .gap(4.0)
            .items_center()
            .child(super::super::icons::draw(
                &super::super::icons::AUDIO_LINES,
                18.0,
                p.soft,
            ))
            .child(super::super::icons::draw(
                if open {
                    &super::super::icons::CHEVRON_DOWN
                } else {
                    &super::super::icons::CHEVRON_UP
                },
                12.0,
                p.soft,
            )),
    )
    .key("meeting-transcript")
    .label(label)
    .px(12.0)
    .h(38.0)
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
        .padding(3.0)
        .radius(24.0)
        .fill(p.card)
        .border(1.0, p.line)
        .child(transcript);
    if recording {
        control = control.child(
            pressable(
                row().items_center().justify_center().child(
                    el().key("meeting-pause-square")
                        .w(12.0)
                        .h(12.0)
                        .radius(2.0)
                        .fill(p.ink),
                ),
            )
            .key("meeting-pause")
            .label("Pause recording")
            .w(34.0)
            .h(34.0)
            .radius(17.0)
            .fill(p.chip)
            .button_disabled(s.hex_record_busy())
            .on_press(|s: &mut State| s.meeting_pause()),
        );
    } else if m.is_some() {
        control = control.child(
            quiet_button("Resume", "meeting-resume", p)
                .button_disabled(s.hex_record_busy())
                .on_press(|s: &mut State| s.meeting_resume()),
        );
    }
    if let Some(m) = m
        && m.phase != SavedPhase::Ended
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
    let m=session(s)?;
    if m.phase==SavedPhase::Ended { return None; }
    let phase=s.live.as_ref().filter(|live| live.session.as_ref().is_some_and(|session|session.id==m.id))
        .map(|live|live.phase).unwrap_or(if m.phase==SavedPhase::Recording {Phase::Recording} else {Phase::Paused});
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

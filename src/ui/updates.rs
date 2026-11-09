//! Quiet update notices and the device's Updates settings.
use super::{
    Event, State, hex,
    theme::Palette,
    widgets::{hairline, label, link, pressable},
};
use rune::{El, col, el, row};
use sidenote::updates::{self, Phase, Status};

pub struct Updates {
    pub status: Status,
    service: Option<std::sync::mpsc::Sender<()>>,
    dismissed: bool,
    notes_open: bool,
    scene: bool,
}
impl Default for Updates {
    fn default() -> Self {
        Self {
            status: updates::load(),
            service: None,
            dismissed: false,
            notes_open: false,
            scene: false,
        }
    }
}
impl State {
    pub fn updates_start(&mut self, waker: rune::Waker) {
        let events = self.events_tx.clone();
        self.updates.service = Some(updates::start(move |status| {
            let _ = events.send(Event::Update(status));
            waker();
        }));
    }
    pub fn update_status(&mut self, status: Status) {
        if self.updates.status.version != status.version && !status.version.is_empty() {
            self.updates.dismissed = false;
        }
        self.updates.status = status;
    }
    fn check_updates(&mut self) {
        if self.updates.scene {
            return;
        }
        if updates::feed().is_none() {
            self.updates.status.phase = Phase::Error;
            self.updates.status.error =
                Some("Choose an update folder with SIDENOTE_UPDATE_FEED, then try again.".into());
            return;
        }
        if let Some(service) = &self.updates.service {
            self.updates.status.phase = Phase::Checking;
            self.updates.status.error = None;
            if service.send(()).is_err() {
                self.updates.status.phase = Phase::Error;
                self.updates.status.error =
                    Some("The update check stopped. Reopen Sidenote to try again.".into());
            }
        }
    }
    fn install_update(&mut self) {
        if self.updates.scene || rune::tooling::headless_run().is_some() {
            return;
        }
        if self.hex_recording()
            || self.hex_record_busy()
            || !self.asks.is_empty()
            || self.live.is_some()
        {
            self.updates.status.error =
                Some("Finish recording and wait for your jobs before installing.".into());
            return;
        }
        if let Err(error) = updates::apply(true, Vec::new()) {
            self.updates.status.error = Some(error);
        }
    }
    fn open_updates(&mut self) {
        self.open_hex();
        self.hex.section = hex::Section::Updates;
        self.hex.filter = rune::TextEdit::default();
    }
}
fn busy(status: &Status) -> bool {
    matches!(
        status.phase,
        Phase::Checking | Phase::Downloading | Phase::Available
    )
}
fn install(s: &State, p: Palette, footer: bool) -> El {
    let ready = s.updates.status.phase == Phase::Ready;
    let b = pressable(
        label(
            "Install and restart",
            13.5,
            600.0,
            if ready { p.card } else { p.faint },
        )
        .px(12.0)
        .py(7.0),
    )
    .key(if footer {
        "update-footer-install"
    } else {
        "update-install"
    })
    .label("Install update and restart Sidenote")
    .disabled(!ready)
    .radius(7.0)
    .fill(if ready { p.accent } else { p.chip });
    if ready {
        b.on_press(|s: &mut State| s.install_update())
    } else {
        b.help("The download must finish first")
    }
}
fn actions(s: &State, p: Palette, footer: bool) -> El {
    let mut actions = row().w_full().flex_wrap().gaps(10.0, 12.0).items_center();
    if matches!(
        s.updates.status.phase,
        Phase::Available | Phase::Downloading | Phase::Ready
    ) {
        actions = actions.child(install(s, p, footer)).child(
            link("Later", p)
                .key(if footer {
                    "update-footer-later"
                } else {
                    "update-later"
                })
                .on_press(|s: &mut State| s.updates.dismissed = true),
        );
    }
    if !s.updates.status.notes.is_empty() {
        actions = actions.child(
            link(
                if s.updates.notes_open {
                    "Hide what changed"
                } else {
                    "What changed"
                },
                p,
            )
            .key(if footer {
                "update-footer-notes"
            } else {
                "update-notes"
            })
            .on_press(|s: &mut State| s.updates.notes_open = !s.updates.notes_open),
        );
    }
    actions
}
pub fn section(s: &State, p: Palette) -> El {
    let status = &s.updates.status;
    let mut body = col()
        .key("updates-section")
        .w_full()
        .gap(12.0)
        .pt(20.0)
        .child(label("Updates", 21.0, 600.0, p.ink))
        .child(label(
            format!("Current version: Sidenote {}", updates::VERSION),
            14.0,
            500.0,
            p.ink,
        ));
    let title = match status.phase {
        Phase::Idle => {
            if status.checked.is_some() {
                "Sidenote is up to date.".into()
            } else {
                "Check for a new version.".into()
            }
        }
        Phase::Checking => "Checking the update folder...".into(),
        Phase::Available | Phase::Downloading | Phase::Ready => {
            format!("Update available: {}", status.version)
        }
        Phase::Error => "Could not update Sidenote.".into(),
    };
    let mut card = col()
        .w_full()
        .gap(10.0)
        .px(18.0)
        .py(18.0)
        .radius(10.0)
        .border(1.0, p.line)
        .fill(p.card)
        .child(label(title, 16.0, 600.0, p.ink).role("status"));
    match status.phase {
        Phase::Available => {
            card = card.child(label(
                "The download will start in the background.",
                13.5,
                400.0,
                p.soft,
            ));
        }
        Phase::Downloading => {
            let width =
                (s.app_w.get() - if s.narrow.get() { 140.0 } else { 430.0 }).clamp(180.0, 720.0);
            card = card
                .child(label(
                    format!("Downloading: {}%", status.progress),
                    13.5,
                    500.0,
                    p.soft,
                ))
                .child(
                    el().w(width)
                        .max_w(width)
                        .h(5.0)
                        .radius(3.0)
                        .fill(p.line)
                        .child(
                            el().w(width * f32::from(status.progress) / 100.0)
                                .h(5.0)
                                .radius(3.0)
                                .fill(p.accent),
                        )
                        .role("progressbar")
                        .range_value(f64::from(status.progress), 0.0, 100.0)
                        .label(format!("Update download {} percent", status.progress)),
                );
        }
        Phase::Ready => {
            card = card.child(label("Ready to install. Restart now, or keep working and install next time you open Sidenote.", 13.5, 400.0, p.soft));
        }
        _ => {}
    }
    if let Some(error) = &status.error {
        card = card.child(label(error, 13.5, 500.0, p.warn).role("alert"));
    }
    if status.phase == Phase::Error || status.error.is_some() {
        card = card.child(
            link("Try again", p)
                .key("update-retry")
                .on_press(|s: &mut State| {
                    if s.updates.status.phase == Phase::Ready {
                        s.install_update();
                    } else {
                        s.check_updates();
                    }
                }),
        );
    }
    card = card.child(actions(s, p, false));
    if s.updates.dismissed {
        card = card.child(label(
            "You can keep working. The update will wait until the next restart.",
            13.0,
            400.0,
            p.soft,
        ));
    }
    if s.updates.notes_open && !status.notes.is_empty() {
        card = card
            .child(hairline(p))
            .child(label("What changed", 14.0, 600.0, p.ink))
            .child(label(&status.notes, 13.5, 400.0, p.soft));
    }
    body = body.child(card).child(label(
        format!(
            "Last checked: {}",
            status
                .checked
                .as_deref()
                .and_then(|v| chrono::DateTime::parse_from_rfc3339(v).ok())
                .map_or_else(
                    || "Not yet".into(),
                    |d| d.format("%d %b %Y at %H:%M").to_string()
                )
        ),
        13.0,
        400.0,
        p.soft,
    ));
    body = body
        .child(label("Update folder", 13.0, 600.0, p.soft))
        .child(label(
            if s.updates.scene {
                "E:\\Sidenote updates".into()
            } else {
                updates::source_label()
            },
            13.5,
            400.0,
            p.ink,
        ));
    if updates::feed().is_none() && !s.updates.scene {
        body = body.child(label(
            "Set SIDENOTE_UPDATE_FEED in your launcher to choose a folder or file share.",
            13.0,
            400.0,
            p.soft,
        ));
    }
    if !busy(status) {
        body = body.child(
            link("Check for updates", p)
                .key("update-check")
                .on_press(|s: &mut State| s.check_updates()),
        );
    }
    body.child(label("Checks run when Sidenote opens and every 6 hours. Installation waits until recording, Ask and background jobs finish.", 13.0, 400.0, p.faint).mt(8.0))
}
pub fn footer(s: &State, p: Palette, wide: bool) -> Option<El> {
    let u = &s.updates;
    if u.dismissed || u.status.version.is_empty() {
        return None;
    }
    if !wide {
        return Some(
            link("↑", p)
                .key("update-footer")
                .label("Update available. Open Updates settings")
                .on_press(|s: &mut State| s.open_updates()),
        );
    }
    Some(
        col()
            .w_full()
            .gap(5.0)
            .py(10.0)
            .child(
                link(format!("Update available: {}", u.status.version), p)
                    .key("update-footer")
                    .on_press(|s: &mut State| s.open_updates()),
            )
            .child(actions(s, p, true)),
    )
}
pub fn scene(s: &mut State, phase: Phase) {
    hex::scene(s, false);
    s.open_updates();
    s.updates.scene = true;
    s.updates.status = Status { version: if phase == Phase::Idle { String::new() } else { "0.2.0".into() },
        progress: if phase == Phase::Downloading { 42 } else { 100 },
        notes: "Updates now download in the background. Install when you are ready.\nYour meetings and notes stay on this device.".into(),
        checked: Some("2026-11-09T12:40:00+05:30".into()),
        error: (phase == Phase::Error).then(|| "Could not verify the update. Your current version is still running.".into()),
        phase };
    if s.updates.status.phase == Phase::Idle {
        s.updates.status.notes.clear();
    }
    s.updates.notes_open = s.updates.status.phase == Phase::Ready;
}

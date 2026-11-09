//! Quiet update notices and the device's Updates settings.
use super::{
    Event, State, hex, roles,
    theme::Palette,
    widgets::{hairline, link, pressable},
};
use rune::{El, col, el, row};
use sidenote::updates::{self, Phase, Status};

pub struct Updates {
    pub status: Status,
    service: Option<std::sync::mpsc::Sender<updates::CheckRequest>>,
    dismissed: bool,
    notes_open: bool,
    scene: bool,
    applying: bool,
}
impl Default for Updates {
    fn default() -> Self {
        Self {
            status: updates::load(),
            service: None,
            dismissed: false,
            notes_open: false,
            scene: false,
            applying: false,
        }
    }
}
impl State {
    pub fn updates_start(&mut self) {
        if !updates::installed() {
            return;
        }
        self.updates.applying = true;
        let events = self.events_tx.clone();
        let wake = self.waker.clone();
        let status = self.updates.status.clone();
        std::thread::spawn(move || {
            let result = updates::window_start(|phase| {
                let _ = events.send(Event::Update(Status {
                    phase,
                    ..status.clone()
                }));
                if let Some(wake) = &wake {
                    wake();
                }
            });
            let _ = events.send(Event::UpdateFinished(result.map(|_| updates::load())));
            if let Some(wake) = &wake {
                wake();
            }
        });
    }
    pub fn update_status(&mut self, status: Status) {
        let preparing = matches!(status.phase, Phase::Preparing | Phase::StoppingListener);
        if self.updates.applying && !preparing {
            return;
        }
        if preparing {
            self.updates.applying = true;
        }
        if self.updates.status.version != status.version && !status.version.is_empty() {
            self.updates.dismissed = false;
        }
        self.updates.status = status;
    }
    pub fn update_finished(&mut self, result: Result<Status, String>) {
        self.updates.applying = false;
        if self.updates.service.is_none() && updates::installed() {
            let events = self.events_tx.clone();
            let wake = self.waker.clone();
            self.updates.service = updates::start(move |status| {
                let _ = events.send(Event::Update(status));
                if let Some(wake) = &wake {
                    wake();
                }
            });
        }
        match result {
            Ok(status) => self.updates.status = status,
            Err(error) => {
                self.updates.status.phase = if self.updates.status.version.is_empty() {
                    Phase::Error
                } else {
                    Phase::Ready
                };
                self.updates.status.error = Some(error);
            }
        }
    }
    fn check_updates(&mut self) {
        if self.updates.scene
            || self.updates.applying
            || self.updates.status.phase == Phase::Manual
            || busy(&self.updates.status)
        {
            return;
        }
        if let Some(service) = &self.updates.service {
            self.updates.status.phase = Phase::Checking;
            self.updates.status.error = None;
            if service.send(updates::CheckRequest::Manual).is_err() {
                self.updates.status.phase = Phase::Error;
                self.updates.status.error =
                    Some("The update check stopped. Reopen Sidenote to try again.".into());
            }
        }
    }
    pub fn updates_focus(&mut self) {
        if !updates::installed()
            || self.updates.scene
            || self.updates.applying
            || busy(&self.updates.status)
            || !self
                .updates
                .status
                .checked
                .as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .is_some_and(|checked| {
                    chrono::Utc::now().signed_duration_since(checked)
                        > chrono::Duration::seconds(updates::FOCUS_CHECK_AFTER.as_secs() as i64)
                })
        {
            return;
        }
        if let Some(service) = &self.updates.service {
            let _ = service.send(updates::CheckRequest::Focus);
        }
    }
    fn install_update(&mut self) {
        if self.updates.scene
            || self.updates.applying
            || self.updates.status.phase != Phase::Ready
            || rune::tooling::headless_run().is_some()
        {
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
        self.updates.applying = true;
        self.updates.status.phase = Phase::Preparing;
        self.updates.status.error = None;
        let status = self.updates.status.clone();
        let events = self.events_tx.clone();
        let wake = self.waker.clone();
        std::thread::spawn(move || {
            let result = updates::apply(true, Vec::new(), |phase| {
                let _ = events.send(Event::Update(Status {
                    phase,
                    ..status.clone()
                }));
                if let Some(wake) = &wake {
                    wake();
                }
            });
            let _ = events.send(Event::UpdateFinished(result.map(|_| updates::load())));
            if let Some(wake) = &wake {
                wake();
            }
        });
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
        Phase::Checking
            | Phase::Downloading
            | Phase::Available
            | Phase::Preparing
            | Phase::StoppingListener
    )
}
fn install(s: &State, p: Palette, footer: bool) -> El {
    let ready = s.updates.status.phase == Phase::Ready;
    let b = pressable(
        roles::CONTROL
            .wrap(
                match s.updates.status.phase {
                    Phase::Downloading => format!("Downloading {}%", s.updates.status.progress),
                    Phase::Preparing => "Preparing update".into(),
                    Phase::StoppingListener => "Stopping the listener".into(),
                    _ => "Install and restart".into(),
                },
                if ready { p.card } else { p.soft },
            )
            .px(12.0)
            .py(7.0),
    )
    .key(if footer {
        "update-footer-install"
    } else {
        "update-install"
    })
    .label(if ready {
        "Install update and restart Sidenote"
    } else {
        "Update progress"
    })
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
        Phase::Available
            | Phase::Downloading
            | Phase::Ready
            | Phase::Preparing
            | Phase::StoppingListener
    ) {
        if s.updates.status.phase != Phase::Available {
            actions = actions.child(install(s, p, footer));
        }
        if !s.updates.applying
            && !matches!(
                s.updates.status.phase,
                Phase::Preparing | Phase::StoppingListener
            )
        {
            actions = actions.child(
                link("Later", p)
                    .key(if footer {
                        "update-footer-later"
                    } else {
                        "update-later"
                    })
                    .on_press(|s: &mut State| s.updates.dismissed = true),
            );
        }
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
    if s.updates.status.phase == Phase::Manual {
        return col()
            .w_full()
            .gap(12.0)
            .pt(20.0)
            .child(roles::SECTION.wrap("Updates", p.ink))
            .child(roles::SMALL.wrap("Updates are off for this manual install", p.soft));
    }
    let status = &s.updates.status;
    let mut body = col()
        .key("updates-section")
        .w_full()
        .gap(12.0)
        .pt(20.0)
        .child(roles::SECTION.wrap("Updates", p.ink))
        .child(roles::CONTROL.wrap(
            format!("Current version: Sidenote {}", updates::VERSION),
            p.ink,
        ));
    let title = match status.phase {
        Phase::Manual => "Updates are off for this manual install".into(),
        Phase::Preparing => "Preparing update".into(),
        Phase::StoppingListener => "Stopping the listener".into(),
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
        .child(roles::TITLE.wrap(title, p.ink).role("status"));
    match status.phase {
        Phase::Available => {
            card =
                card.child(roles::SMALL.wrap("The download will start in the background.", p.soft));
        }
        Phase::Downloading => {
            let width =
                (s.app_w.get() - if s.narrow.get() { 140.0 } else { 430.0 }).clamp(180.0, 720.0);
            card = card
                .child(roles::CONTROL.wrap(format!("Downloading: {}%", status.progress), p.soft))
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
            card = card.child(roles::SMALL.wrap("Ready to install. Restart now, or keep working and install next time you open Sidenote.", p.soft));
        }
        _ => {}
    }
    if let Some(error) = &status.error {
        card = card.child(roles::CONTROL.wrap(error, p.warn).role("alert"));
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
        card = card.child(roles::META.wrap(
            "You can keep working. The update will wait until the next restart.",
            p.soft,
        ));
    }
    if s.updates.notes_open && !status.notes.is_empty() {
        card = card
            .child(hairline(p))
            .child(roles::SMALL.bold().wrap("What changed", p.ink))
            .child(roles::SMALL.wrap(&status.notes, p.soft));
    }
    body = body.child(card).child(roles::META.wrap(
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
        p.soft,
    ));
    body = body
        .child(roles::GROUP.wrap("Update source", p.soft))
        .child(roles::SMALL.wrap(if s.updates.scene {
                "E:\\Sidenote updates".into()
            } else {
                updates::source_label()
            }, p.ink))
        .child(roles::SMALL.wrap(if updates::feed().is_some() {
            "Anyone who can change files in this folder can change what gets installed. Use a folder only you can write to."
        } else {
            "Updates download from Sidenote's public GitHub Releases over HTTPS."
        }, p.soft));
    if !busy(status) {
        body = body.child(
            link("Check for updates", p)
                .key("update-check")
                .on_press(|s: &mut State| s.check_updates()),
        );
    }
    body.child(roles::META.wrap("Checks run when Sidenote opens, every 6 hours, and on focus if the last attempt was over 30 minutes ago. Installation waits until recording, Ask and background jobs finish.", p.faint).mt(8.0))
}
pub fn footer(s: &State, p: Palette, wide: bool) -> Option<El> {
    let u = &s.updates;
    if u.dismissed || u.status.phase != Phase::Ready {
        return None;
    }
    if !wide {
        return Some(
            link("?", p)
                .key("update-footer")
                .label("Update ready. Open Updates settings")
                .on_press(|s: &mut State| s.open_updates()),
        );
    }
    Some(
        col()
            .w_full()
            .gap(3.0)
            .py(10.0)
            .child(roles::CONTROL.line(format!("Sidenote {} is ready", u.status.version), p.soft))
            .child(
                link("Restart to update", p)
                    .key("update-footer-install")
                    .on_press(|s: &mut State| s.install_update()),
            ),
    )
}
pub fn scene(s: &mut State, phase: Phase) {
    hex::scene(s, false);
    s.open_updates();
    s.updates.scene = true;
    s.updates.status = Status { version: if phase == Phase::Idle { String::new() } else { "0.2.0".into() },
        progress: if phase == Phase::Downloading { 40 } else { 100 },
        notes: "Updates now download in the background. Install when you are ready.\nYour meetings and notes stay on this device.".into(),
        checked: Some("2026-11-09T12:40:00+05:30".into()),
        error: (phase == Phase::Error).then(|| "Could not verify the update. Your current version is still running.".into()),
        phase };
    if s.updates.status.phase == Phase::Idle {
        s.updates.status.notes.clear();
    }
    s.updates.notes_open = s.updates.status.phase == Phase::Ready;
}
pub fn settings_scene(s: &mut State) {
    s.open_updates();
}

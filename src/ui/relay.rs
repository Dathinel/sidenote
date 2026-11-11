use super::{
    Event, State,
    hex::settings::{card, setting_row},
    roles,
    theme::Palette,
    widgets::pressable,
};
use rune::{El, TextEdit, Tx, col, row};
use rune_kit::{Kit, TextInput};
use sidenote::core::relay_preview::{
    self as core, Conflict,
    pairing::{PairedDevice, Pairing, Update},
};
use sidenote_sync::pairing::SecretCode;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct RelayUi {
    pub fixture: bool,
    job: Option<Pairing>,
    generation: u64,
    pub code: Option<SecretCode>,
    pub expires: Option<Instant>,
    pub confirmation: Option<(String, String, String)>,
    pub join: TextEdit,
    pub peers: Vec<PairedDevice>,
    pub conflicts: Vec<Conflict>,
    pub resolutions: Vec<core::Resolution>,
    pub error: Option<String>,
    pub removing: Option<String>,
    pub waiting: bool,
}

fn button(label: &str, key: impl Into<String>, p: Palette) -> El {
    pressable(roles::CONTROL.wrap(label, p.ink))
        .key(key.into())
        .label(label)
        .padding(10.0)
        .radius(7.0)
        .fill(p.chip)
        .border(1.0, p.line)
        .hover(p.sel)
}

impl State {
    pub fn relay_refresh(&mut self) {
        if self.relay_ui.fixture {
            return;
        }
        if let Ok(settings) = core::Settings::load(&self.store.root) {
            self.hex.shell.relay = settings;
        }
        if !self.hex.shell.relay.enabled {
            self.relay_ui.peers.clear();
            self.relay_ui.conflicts.clear();
            return;
        }
        if let Ok(peers) = core::pairing::devices(&self.store.root) {
            self.relay_ui.peers = peers;
        }
        if let Ok(conflicts) = core::conflicts(&self.store.root) {
            self.relay_ui.conflicts = conflicts;
        }
        if let Ok(Some(preview)) = core::get(&self.store.root)
            && let Ok(records) = preview.resolutions()
        {
            self.relay_ui.resolutions = records;
        }
    }
    pub fn relay_cancel(&mut self) {
        self.relay_ui.generation += 1;
        self.relay_ui.job.take();
        self.relay_ui.code = None;
        self.relay_ui.expires = None;
        self.relay_ui.confirmation = None;
        self.relay_ui.waiting = false;
        self.relay_ui.join = TextEdit::default();
    }
    fn relay_pair(&mut self, joining: bool) {
        let code = joining.then(|| SecretCode::new(std::mem::take(&mut self.relay_ui.join.text)));
        self.relay_cancel();
        self.relay_ui.error = None;
        match core::pairing::start(&self.store.root, code) {
            Ok((job, updates)) => {
                self.relay_ui.job = Some(job);
                self.relay_ui.waiting = true;
                let generation = self.relay_ui.generation;
                let events = self.events_tx.clone();
                let waker = self.waker.clone();
                std::thread::spawn(move || {
                    for update in updates {
                        if events
                            .send(Event::RelayPair { generation, update })
                            .is_err()
                        {
                            break;
                        }
                        if let Some(waker) = &waker {
                            waker();
                        }
                    }
                });
            }
            Err(_) => {
                self.relay_ui.error =
                    Some("Save a valid relay address in Advanced, then try pairing again.".into())
            }
        }
    }
    fn relay_confirm(&mut self, accept: bool) {
        if !accept {
            self.relay_cancel();
            return;
        }
        if let Some(job) = &self.relay_ui.job {
            job.confirm(true);
            self.relay_ui.confirmation = None;
            self.relay_ui.waiting = true;
        }
    }
    pub fn relay_update(&mut self, generation: u64, update: Update) {
        if self.relay_ui.generation != generation {
            return;
        }
        match update {
            Update::Code { code, expires } => {
                self.relay_ui.code = Some(code);
                self.relay_ui.expires = Some(expires);
                self.relay_ui.waiting = false;
            }
            Update::Confirm {
                local_name,
                peer_name,
                phrase,
            } => {
                self.relay_ui.code = None;
                self.relay_ui.confirmation = Some((local_name, peer_name, phrase));
                self.relay_ui.waiting = false;
            }
            Update::Ready => {
                self.relay_cancel();
                if let Ok(settings) = core::Settings::load(&self.store.root) {
                    self.hex.shell.relay = settings;
                }
                self.relay_refresh();
                self.request_refresh();
                self.toast = Some((
                    "Device paired. Manual tasks will sync when both devices are online.".into(),
                    Instant::now(),
                ));
            }
            Update::Failed => {
                self.relay_cancel();
                self.relay_ui.error=Some("Pairing was refused, expired or interrupted. Check the code and both confirmations, then create a new code.".into());
            }
        }
    }
}

pub fn tick(s: &mut State, tx: &mut Tx) -> bool {
    if let Some(expires) = s.relay_ui.expires {
        if Instant::now() >= expires {
            s.relay_cancel();
            s.relay_ui.error = Some("The code expired. Create a new code.".into());
            return true;
        }
        tx.wake_after(expires.saturating_duration_since(Instant::now()));
    }
    false
}

pub fn section(s: &State, k: &Kit, p: Palette) -> El {
    if !s.hex.shell.relay.enabled {
        return col();
    }
    let mut out=col().w_full().gap(8.0)
        .child(roles::SMALL.wrap("Tasks you add by hand sync through the relay; everything else still syncs through your folder.",p.ink))
        .child(roles::META.wrap("A verified manual-task backup is kept on this PC. Turning preview off restores the folder view and keeps relay data for later.",p.soft));
    for conflict in &s.relay_ui.conflicts {
        out = out.child(chooser(s, conflict, p));
    }
    if !s.relay_ui.resolutions.is_empty() {
        let mut history = col().w_full().gap(8.0).padding(14.0);
        for record in s.relay_ui.resolutions.iter().rev().take(3) {
            let value = record
                .chosen
                .value
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| record.chosen.value.to_string());
            history = history.child(roles::SMALL.wrap(
                format!(
                    "{} chose {} for {}. {}. Both earlier versions are kept.",
                    record.device, value, record.field, record.at
                ),
                p.soft,
            ));
        }
        out = out.child(card("Recent task choices", history, p));
    }
    let mut pair = col().w_full().gap(12.0).padding(14.0);
    if let Some((local, peer, phrase)) = &s.relay_ui.confirmation {
        pair=pair.child(roles::NAME.wrap("Confirm on both devices",p.ink))
            .child(roles::CONTROL.wrap(format!("This device: {local}    Other device: {peer}"),p.soft))
            .child(roles::SECTION.wrap(phrase,p.ink).key("relay-safety-phrase"))
            .child(roles::SMALL.wrap("Only confirm if these words and both device names match the other screen. No tasks flow to the new device until both sides confirm.",p.soft))
            .child(row().gap(8.0).child(button("They match. Confirm","relay-confirm",p).on_press(|s:&mut State|s.relay_confirm(true)))
                .child(button("Do not pair","relay-reject",p).on_press(|s:&mut State|s.relay_confirm(false))));
    } else if let Some(code) = &s.relay_ui.code {
        let shown = format!(
            "{} {}",
            code.chars().take(4).collect::<String>(),
            code.chars().skip(4).collect::<String>()
        );
        pair=pair.child(roles::NAME.wrap("Enter this code on the other device",p.ink))
            .child(roles::SECTION.wrap(shown,p.ink).key("relay-invitation-code"))
            .child(roles::SMALL.wrap("This code expires after 10 minutes and works once. Both devices must also confirm the matching safety words.",p.soft))
            .child(button("Cancel pairing","relay-cancel",p).self_start().on_press(|s:&mut State|s.relay_cancel()));
    } else if s.relay_ui.waiting {
        pair = pair
            .child(roles::CONTROL.wrap(
                "Waiting for the other device and its confirmation...",
                p.soft,
            ))
            .child(
                button("Cancel pairing", "relay-cancel", p)
                    .self_start()
                    .on_press(|s: &mut State| s.relay_cancel()),
            );
    } else {
        pair = pair
            .child(roles::SMALL.wrap(
                "Create a code here, or enter the code shown on your other device.",
                p.soft,
            ))
            .child(
                button("Show a pairing code", "relay-host", p)
                    .self_start()
                    .on_press(|s: &mut State| s.relay_pair(false)),
            )
            .child(
                row()
                    .gap(8.0)
                    .items_center()
                    .child(
                        TextInput::new(&s.relay_ui.join)
                            .label("Code from the other device")
                            .placeholder("8-digit code")
                            .on_edit(|s: &mut State| &mut s.relay_ui.join)
                            .on_submit(|s: &mut State, _| s.relay_pair(true))
                            .build(k)
                            .key("relay-join-code")
                            .w(220.0),
                    )
                    .child(
                        button("Pair a device", "relay-join", p)
                            .on_press(|s: &mut State| s.relay_pair(true)),
                    ),
            );
    }
    out = out.child(card("Pair a device", pair, p));
    if !s.relay_ui.peers.is_empty() {
        let mut peers = col().w_full();
        for (index, peer) in s.relay_ui.peers.iter().enumerate() {
            let control = if peer.local {
                roles::SMALL.wrap("This device", p.soft)
            } else {
                let target = peer.token.clone();
                button("Remove", format!("relay-remove-{index}"), p)
                    .on_press(move |s: &mut State| s.relay_ui.removing = Some(target.clone()))
            };
            peers = peers.child(setting_row(
                s,
                &format!("relay-paired-device-{index}"),
                &peer.name,
                "Paired for manual-task sync.",
                control,
                peer.local,
                p,
            ));
        }
        if let Some(target) = &s.relay_ui.removing {
            let target = target.clone();
            peers=peers.child(col().padding(14.0).gap(8.0)
                .child(roles::SMALL.wrap("Remove this device and rotate the group key? Future sync stops. Other devices must pair again. Data already on a device stays there.",p.warn))
                .child(button("Remove and require re-pairing","relay-remove-confirm",p).self_start().on_press(move|s:&mut State| {
                    s.relay_cancel();
                    let result=core::get(&s.store.root).and_then(|p|p.ok_or_else(||std::io::Error::other("preview off"))).and_then(|p|p.remove_device(&target));
                    s.relay_ui.removing=None;
                    if result.is_err(){s.relay_ui.error=Some("Could not remove the device. Try again.".into());}
                    s.relay_refresh();
                })));
        }
        out = out.child(card("Paired devices", peers, p));
    }
    if let Some(error) = &s.relay_ui.error {
        out = out.child(roles::SMALL.wrap(error, p.warn).role("alert"));
    }
    out
}

fn chooser(_s: &State, conflict: &Conflict, p: Palette) -> El {
    let mut body = col()
        .w_full()
        .gap(10.0)
        .padding(14.0)
        .child(roles::NAME.wrap(
            format!("{} · {} versions", conflict.title, conflict.choices.len()),
            p.ink,
        ))
        .child(roles::SMALL.wrap(
            format!(
                "Choose the {} to keep. Both earlier values stay in history.",
                conflict.field.replace('_', " ")
            ),
            p.soft,
        ));
    for (index, choice) in conflict.choices.iter().enumerate() {
        let (id, field, chosen) = (
            conflict.task_id.clone(),
            conflict.field.clone(),
            choice.clone(),
        );
        let time = if choice.at.is_empty() {
            "Time not recorded"
        } else {
            &choice.at
        };
        let value = choice
            .value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| choice.value.to_string());
        body = body.child(
            super::cards::panel(p).child(
                col()
                    .w_full()
                    .gap(6.0)
                    .padding(12.0)
                    .child(roles::SMALL.wrap(format!("{} · {time}", choice.device), p.soft))
                    .child(roles::CONTROL.wrap(value, p.ink))
                    .child(
                        button(
                            "Use this version",
                            format!(
                                "relay-use-version-{}-{}-{index}",
                                conflict.task_id, conflict.field
                            ),
                            p,
                        )
                        .self_start()
                        .on_press(move |s: &mut State| {
                            let result = core::get(&s.store.root)
                                .and_then(|p| p.ok_or_else(|| std::io::Error::other("preview off")))
                                .and_then(|p| p.resolve(&id, &field, &chosen));
                            if result.is_ok() {
                                s.toast = Some((
                                    "Choice saved. Both earlier versions remain in history.".into(),
                                    Instant::now(),
                                ));
                                s.relay_refresh();
                                s.request_refresh();
                            } else {
                                s.relay_ui.error =
                                    Some("Versions changed. Refresh and choose again.".into());
                            }
                        }),
                    ),
            ),
        );
    }
    card("Resolve task versions", body, p).key(format!(
        "relay-conflict-chooser-{}-{}",
        conflict.task_id, conflict.field
    ))
}

pub fn task_chooser(s: &State, task: &sidenote::core::TaskItem, p: Palette) -> Option<El> {
    let id = task.meeting_id.strip_prefix("manual/")?;
    let items: Vec<_> = s
        .relay_ui
        .conflicts
        .iter()
        .filter(|c| c.task_id == id)
        .collect();
    if items.is_empty() {
        None
    } else {
        Some(
            col()
                .w_full()
                .children(items.into_iter().map(|c| chooser(s, c, p))),
        )
    }
}

pub fn scene(s: &mut State, kind: &str) {
    assert!(super::isolated());
    super::hex::settings::scene(s, "settings-sync");
    s.relay_ui.fixture = true;
    s.hex.shell.relay.enabled = true;
    s.hex.shell.relay.address = "ws://127.0.0.1:8787".into();
    s.hex.shell.relay_address = TextEdit::new("ws://127.0.0.1:8787");
    s.relay_ui.peers = vec![PairedDevice {
        token: "synthetic-local".into(),
        name: "DEMO-PC".into(),
        local: true,
    }];
    match kind {
        "code" => {
            let sample = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("render clock")
                .subsec_nanos()
                % 100_000_000;
            s.relay_ui.code = Some(SecretCode::new(format!("{sample:08}")));
            s.relay_ui.expires = Some(Instant::now() + Duration::from_secs(600));
        }
        "confirm" => {
            s.relay_ui.confirmation = Some((
                "DEMO-PC".into(),
                "Travel laptop".into(),
                "amber birch cedar dawn elm fern glen harbor".into(),
            ))
        }
        "paired" => s.relay_ui.peers.push(PairedDevice {
            token: "synthetic-peer".into(),
            name: "Travel laptop".into(),
            local: false,
        }),
        "conflict" => {
            core::seed_render_conflict(&s.store.root).expect("isolated synthetic conflict");
            s.hex.shell.relay =
                core::Settings::load(&s.store.root).expect("synthetic relay settings");
            s.relay_ui.fixture = false;
            s.relay_refresh();
        }
        _ => {}
    }
}

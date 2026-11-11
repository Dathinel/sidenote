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
    pub operational: core::operations::Operational,
    pub status_checked: Option<Instant>,
    job: Option<Pairing>,
    generation: u64,
    pub code: Option<SecretCode>,
    pub expires: Option<Instant>,
    pub confirmation: Option<(u64, String, String, String)>,
    pub join: TextEdit,
    pub join_open: bool,
    pub peers: Vec<PairedDevice>,
    pub conflicts: Vec<Conflict>,
    pub resolutions: Vec<core::Resolution>,
    pub error: Option<String>,
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

fn primary_button(label: &str, key: &str, p: Palette) -> El {
    super::tasks::button(label, key, true, p).py(10.0)
}

fn complete_code(text: &str) -> bool {
    text.bytes().filter(|b| b.is_ascii_digit()).count() == 8
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || b.is_ascii_whitespace())
}

fn group_code(edit: &mut TextEdit) {
    let digit_position = |index: usize| {
        edit.text
            .char_indices()
            .take_while(|(i, _)| *i < index)
            .filter(|(_, c)| c.is_ascii_digit())
            .count()
            .min(8)
    };
    let (anchor, focus) = (digit_position(edit.anchor), digit_position(edit.focus));
    let digits = SecretCode::new(
        edit.text
            .chars()
            .filter(char::is_ascii_digit)
            .take(8)
            .collect(),
    );
    let grouped = if digits.len() > 4 {
        format!("{} {}", &digits[..4], &digits[4..])
    } else {
        digits.to_string()
    };
    if grouped != edit.text {
        edit.replace_text(grouped);
        edit.anchor = anchor + usize::from(anchor > 4);
        edit.focus = focus + usize::from(focus > 4);
    }
}

fn human_time(at: &str) -> String {
    human_time_at(at, chrono::Local::now())
}

fn human_time_at(at: &str, now: chrono::DateTime<chrono::Local>) -> String {
    let Ok(time) = chrono::DateTime::parse_from_rfc3339(at) else {
        return "Time not recorded".into();
    };
    let time = time.with_timezone(&chrono::Local);
    let day = match now
        .date_naive()
        .signed_duration_since(time.date_naive())
        .num_days()
    {
        0 => "today".into(),
        1 => "yesterday".into(),
        _ => time.format("%-d %b %Y").to_string(),
    };
    format!("{day}, {}", time.format("%H:%M"))
}

fn field_name(field: &str) -> String {
    let path: Option<Vec<String>> = serde_json::from_str(field).ok();
    let field = path
        .as_ref()
        .and_then(|p| p.last())
        .map(|p| p.strip_prefix("o:").unwrap_or(p))
        .unwrap_or(field);
    let text = field.replace('_', " ");
    let mut chars = text.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
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
        if let Ok(status) = core::operations::status(&self.store.root) {
            self.relay_ui.operational = status;
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
        self.relay_ui.join = TextEdit::default().secure().revealed(true);
        self.relay_ui.join_open = false;
    }
    fn relay_pair(&mut self, joining: bool) {
        if joining && !complete_code(&self.relay_ui.join.text) {
            return;
        }
        let code = joining.then(|| {
            SecretCode::new(
                std::mem::take(&mut self.relay_ui.join.text)
                    .split_whitespace()
                    .collect(),
            )
        });
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
    fn relay_confirm(&mut self, attempt: u64, accept: bool) {
        if self.relay_ui.confirmation.as_ref().map(|info| info.0) != Some(attempt) {
            return;
        }
        if !accept {
            self.relay_cancel();
            return;
        }
        if let Some(job) = &self.relay_ui.job {
            job.confirm(attempt, true);
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
                attempt,
                local_name,
                peer_name,
                phrase,
            } => {
                self.relay_ui.code = None;
                self.relay_ui.confirmation = Some((attempt, local_name, peer_name, phrase));
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
                    "Device paired. Shared data will sync when both devices are online.".into(),
                    Instant::now(),
                ));
            }
            Update::TimedOut => {
                self.relay_cancel();
                self.relay_ui.error = Some(core::pairing::TIMED_OUT.into());
            }
            Update::Unconfirmed => {
                self.relay_cancel();
                self.relay_refresh();
                self.request_refresh();
                self.relay_ui.error = Some(core::pairing::UNCONFIRMED.into());
            }
            Update::Failed => {
                self.relay_cancel();
                self.relay_ui.error=Some("Pairing was refused, expired or interrupted. Check the code and both confirmations, then create a new code.".into());
            }
        }
    }
}

pub fn tick(s: &mut State, tx: &mut Tx) -> bool {
    if s.hex.shell.relay.enabled && !s.relay_ui.fixture {
        tx.wake_after(Duration::from_secs(5));
        if s.relay_ui
            .status_checked
            .is_none_or(|at| at.elapsed() >= Duration::from_secs(5))
        {
            s.relay_ui.status_checked = Some(Instant::now());
            if let Ok(status) = core::operations::status(&s.store.root) {
                s.relay_ui.operational = status;
                return true;
            }
        }
    }
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

pub fn status_banner(s: &State, p: Palette) -> Option<El> {
    if !s.hex.shell.relay.enabled {
        return None;
    }
    let status = &s.relay_ui.operational;
    let icon = if status.read_only {
        &*super::icons::WARNING
    } else if status.message.starts_with("Relay unavailable.") {
        &*super::icons::CLOUD_OFF
    } else if status.message.starts_with("This address")
        || status.message.starts_with("Enter a valid")
    {
        &*super::icons::WARNING
    } else {
        return None;
    };
    Some(super::widgets::status_notice(&status.message, icon, p).key("relay-status-banner"))
}

pub fn section(s: &State, k: &Kit, p: Palette) -> El {
    if !s.hex.shell.relay.enabled {
        return col();
    }
    let mut out = col().w_full().min_w(0.0).gap(8.0);
    if s.relay_ui.operational.read_only {
        return out;
    }
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
                    record.device,
                    value,
                    field_name(&record.field).to_lowercase(),
                    human_time(&record.at)
                ),
                p.soft,
            ));
        }
        out = out.child(card("Recent choices", history, p));
    }
    let mut pair = col().w_full().gap(12.0).padding(14.0);
    if s.relay_ui.error.as_deref() == Some(core::pairing::TIMED_OUT) {
        pair = pair.child(
            roles::CONTROL
                .wrap(core::pairing::TIMED_OUT, p.warn)
                .role("alert")
                .key("relay-timeout-message"),
        );
    }
    if let Some((attempt, local, peer, phrase)) = &s.relay_ui.confirmation {
        let attempt = *attempt;
        pair=pair.child(roles::NAME.wrap("Confirm on both devices",p.ink))
            .child(roles::CONTROL.wrap(format!("This device: {local}    Other device: {peer}"),p.soft))
            .child(roles::SECTION.wrap(phrase,p.ink).key("relay-safety-phrase"))
            .child(roles::SMALL.wrap("Only confirm if these words and both device names match the other screen. No tasks flow to the new device until both sides confirm. If the other device shows an error or no words, choose Do not pair.",p.soft))
            .child(row().gap(8.0).child(button("They match. Confirm","relay-confirm",p).on_press(move |s:&mut State|s.relay_confirm(attempt,true)))
                .child(button("Do not pair","relay-reject",p).on_press(move |s:&mut State|s.relay_confirm(attempt,false))));
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
        pair = pair.child(
            primary_button("Show a pairing code", "relay-host", p)
                .self_start()
                .on_press(|s: &mut State| s.relay_pair(false)),
        );
        if s.relay_ui.join_open {
            let ready = complete_code(&s.relay_ui.join.text);
            let mut join = button("Pair", "relay-join", p).disabled(!ready);
            if ready {
                join = join.on_press(|s: &mut State| s.relay_pair(true));
            } else {
                join = join.dim(0.5);
            }
            pair = pair.child(
                col()
                    .w_full()
                    .gap(8.0)
                    .child(roles::SMALL.wrap("Code from the other device", p.soft))
                    .child(
                        row()
                            .gap(8.0)
                            .items_center()
                            .child(
                                TextInput::new(&s.relay_ui.join)
                                    .secure()
                                    .revealed(true)
                                    .label("Code from the other device")
                                    .placeholder("1234 5678")
                                    .on_edit(|s: &mut State| &mut s.relay_ui.join)
                                    .on_change(|s: &mut State, _| group_code(&mut s.relay_ui.join))
                                    .on_submit(|s: &mut State, _| s.relay_pair(true))
                                    .build(k)
                                    .key("relay-join-code")
                                    .w(220.0),
                            )
                            .child(join),
                    ),
            );
        } else {
            pair = pair.child(
                super::widgets::link("Have a code from another device? Enter it", p)
                    .key("relay-enter-code")
                    .self_start()
                    .on_press(|s: &mut State| {
                        s.relay_ui.join_open = true;
                        s.relay_ui.join = TextEdit::default().secure().revealed(true);
                    }),
            );
        }
    }
    out = out.child(card("Pair a device", pair, p).key("relay-pair-card"));
    if !s.relay_ui.peers.is_empty() {
        let mut peers = col().w_full();
        for (index, peer) in s.relay_ui.peers.iter().enumerate() {
            let control = col();
            peers = peers.child(setting_row(
                s,
                &format!("relay-paired-device-{index}"),
                &peer.name,
                if peer.unconfirmed {
                    "Pairing not confirmed"
                } else {
                    "Paired for relay sync."
                },
                control,
                peer.local,
                p,
            ));
        }
        peers = peers.child(super::widgets::hairline(p)).child(
            roles::SMALL
                .wrap("Removing a device is not available in this preview", p.soft)
                .padding(14.0)
                .key("relay-removal-unavailable"),
        );
        if s.relay_ui.peers.iter().any(|peer| peer.unconfirmed)
            || s.relay_ui.error.as_deref() == Some(core::pairing::UNCONFIRMED)
        {
            peers = peers.child(
                roles::SMALL
                    .wrap(core::pairing::UNCONFIRMED, p.warn)
                    .padding(14.0)
                    .key("relay-unconfirmed-notice"),
            );
        }
        out = out.child(card("Paired devices", peers, p).key("relay-peers-card"));
    }
    if let Some(error) = &s.relay_ui.error
        && !matches!(
            error.as_str(),
            core::pairing::TIMED_OUT | core::pairing::UNCONFIRMED
        )
    {
        out = out.child(roles::SMALL.wrap(error, p.warn).role("alert"));
    }
    out
}

fn chooser(_s: &State, conflict: &Conflict, p: Palette) -> El {
    let mut body = col()
        .w_full()
        .gap(8.0)
        .padding(14.0)
        .child(roles::NAME.wrap(
            format!(
                "{} \u{00b7} {} has {} versions",
                conflict.title,
                field_name(&conflict.field),
                conflict.choices.len()
            ),
            p.ink,
        ))
        .child(roles::SMALL.wrap(
            "Choose a value to keep. Both earlier values stay in history.",
            p.soft,
        ));
    for (index, choice) in conflict.choices.iter().enumerate() {
        let (chosen, conflict) = (choice.clone(), conflict.clone());
        let value = choice
            .value
            .as_str()
            .map(str::to_owned)
            .unwrap_or_else(|| choice.value.to_string());
        if index > 0 {
            body = body.child(super::widgets::hairline(p));
        }
        body = body.child(
            row()
                .w_full()
                .items_center()
                .gap(12.0)
                .py(8.0)
                .child(
                    col()
                        .grow(1.0)
                        .min_w(0.0)
                        .gap(4.0)
                        .child(roles::CONTROL.wrap(value, p.ink))
                        .child(roles::SMALL.wrap(
                            format!("{} \u{00b7} {}", choice.device, human_time(&choice.at)),
                            p.soft,
                        )),
                )
                .child(
                    button(
                        "Keep this",
                        format!(
                            "relay-use-version-{}-{}-{index}",
                            conflict.task_id, conflict.field
                        ),
                        p,
                    )
                    .on_press(move |s: &mut State| {
                        let result = core::get(&s.store.root)
                            .and_then(|p| p.ok_or_else(|| std::io::Error::other("preview off")))
                            .and_then(|p| p.resolve_conflict(&conflict, &chosen));
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
        );
    }
    card("Resolve versions", body, p).key(format!(
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
        .filter(|c| c.task_id == id || c.task_id.strip_prefix("$manual:") == Some(id))
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
        unconfirmed: false,
    }];
    match kind {
        "operational" | "unreachable" | "wrong-address" | "update-required" => {
            s.hex.shell.relay.address = "ws://127.0.0.1:8787".into();
            s.hex.shell.relay_address = TextEdit::new(&s.hex.shell.relay.address);
            s.hex.shell.advanced.insert("Sync");
            s.relay_ui.operational=core::operations::Operational {
                read_only:kind=="update-required",
                message:match kind {
                    "unreachable"=>"Relay unavailable. Check the address and your connection. Edits stay on this PC.",
                    "wrong-address"=>"This address did not answer as a Sidenote relay. Check the address in Advanced.",
                    "update-required"=>core::operations::UPDATE_REQUIRED,
                    _=>"Relay reachable. Both devices must be online to exchange changes."
                }.into(),
                last_contact:(kind!="wrong-address").then(||chrono::Utc::now().to_rfc3339()),
            };
            super::scene_scroll(s, "relay-last-contact");
        }
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
                1,
                "DEMO-PC".into(),
                "Travel laptop".into(),
                "amber birch cedar dawn elm fern glen harbor".into(),
            ))
        }
        "timeout" => {
            s.relay_ui.confirmation = Some((
                1,
                "DEMO-PC".into(),
                "Travel laptop".into(),
                "synthetic expired phrase".into(),
            ));
            s.relay_ui.waiting = true;
            s.relay_update(s.relay_ui.generation, Update::TimedOut);
        }
        "unconfirmed" => {
            s.relay_ui.peers.push(PairedDevice {
                token: "synthetic-peer".into(),
                name: "Travel laptop".into(),
                local: false,
                unconfirmed: true,
            });
            s.relay_update(s.relay_ui.generation, Update::Unconfirmed);
            super::scene_scroll(s, "relay-unconfirmed-notice");
        }
        "paired" => s.relay_ui.peers.push(PairedDevice {
            token: "synthetic-peer".into(),
            name: "Travel laptop".into(),
            local: false,
            unconfirmed: false,
        }),
        "conflict" | "shared-conflict" => {
            if kind == "shared-conflict" {
                core::seed_shared_render_conflict(&s.store.root).expect("isolated shared conflict");
            } else {
                core::seed_render_conflict(&s.store.root).expect("isolated synthetic conflict");
            }
            s.hex.shell.relay =
                core::Settings::load(&s.store.root).expect("synthetic relay settings");
            s.relay_ui.fixture = false;
            s.relay_refresh();
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};

    #[test]
    fn code_entry_groups_digits_and_keeps_the_pair_action_guarded() {
        let mut edit = TextEdit::new("1234").secure().revealed(true);
        group_code(&mut edit);
        assert!(
            !complete_code(&edit.text),
            "incomplete code enabled pairing"
        );
        edit.replace_text("12345678");
        group_code(&mut edit);
        assert!(
            edit.text.as_bytes().get(4) == Some(&b' '),
            "code was not grouped"
        );
        assert!(edit.anchor == 9 && edit.focus == 9, "caret left the code");
        assert!(complete_code(&edit.text), "grouped code was rejected");
        edit.replace_text("1234 567");
        group_code(&mut edit);
        assert!(!complete_code(&edit.text), "seven digits enabled pairing");
        assert!(
            !complete_code("1234 567x"),
            "non-digit code enabled pairing"
        );
        assert!(!complete_code("1234 56789"), "nine digits enabled pairing");
    }

    #[test]
    fn grouping_preserves_a_selection_inside_a_pasted_code() {
        let mut edit = TextEdit::new("12345678").secure().revealed(true);
        edit.anchor = 2;
        edit.focus = 6;
        group_code(&mut edit);
        assert!(
            edit.anchor == 2 && edit.focus == 7,
            "selection moved to another digit"
        );
    }

    #[test]
    fn conflict_times_use_local_days_and_minutes() {
        let now = chrono::Local
            .with_ymd_and_hms(2026, 11, 11, 12, 39, 42)
            .single()
            .unwrap();
        assert!(human_time_at(&now.to_rfc3339(), now) == "today, 12:39");
        let yesterday = now - chrono::Duration::days(1);
        assert!(human_time_at(&yesterday.to_rfc3339(), now) == "yesterday, 12:39");
        let utc = now.with_timezone(&chrono::Utc).to_rfc3339();
        let shown = human_time_at(&utc, now);
        assert!(shown == format!("today, {:02}:{:02}", now.hour(), now.minute()));
        assert!(human_time_at("", now) == "Time not recorded");
        assert!(field_name("owner") == "Owner");
    }
}

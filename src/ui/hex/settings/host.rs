use super::{State, card as settings_card, roles};
use crate::ui::{theme::Palette, widgets::action};
use rune::{El, TextEdit, Waker, col, row};
use rune_kit::{Kit, TextInput};
use sidenote::core::speech_host::{self, Host, Monitor, Status};

#[derive(Default)]
pub(super) struct Form {
    loaded: bool,
    url: TextEdit,
    token: TextEdit,
    recipient: Option<String>,
    monitor: Option<Monitor>,
    status: Status,
    error: Option<String>,
}

impl Form {
    fn activate(&mut self, host: Host, wake: Waker) -> Result<(), String> {
        self.monitor = None;
        self.recipient = Some(host.recipient().into());
        self.status = Status::Checking;
        self.monitor = Some(host.monitor(move || wake())?);
        Ok(())
    }

    fn save(&mut self, wake: Waker) -> Result<(), String> {
        let host = self.save_record(&speech_host::settings_path()?)?;
        self.activate(host, wake)
    }

    fn save_record(&mut self, path: &std::path::Path) -> Result<Host, String> {
        let host = Host::save_and_consent(path, &self.url.text, self.token.text.trim().as_bytes())?;
        self.url = TextEdit::new(host.recipient());
        self.token.clear();
        Ok(host)
    }

    fn clear(&mut self) -> Result<(), String> {
        speech_host::clear(&speech_host::settings_path()?, &mut self.monitor)?;
        *self = Self {
            loaded: true,
            ..Self::default()
        };
        self.token.set_secure(true);
        Ok(())
    }
}

pub(super) fn tick(s: &mut State) -> bool {
    let form = &mut s.hex.shell.speech_host;
    if !form.loaded
        && let Some(wake) = s.waker.clone()
    {
        form.loaded = true;
        form.token.set_secure(true);
        let result = speech_host::settings_path().and_then(|path| Host::load(&path));
        form.error = result
            .and_then(|host| {
                if let Some(host) = host {
                    form.url = TextEdit::new(host.recipient());
                    form.activate(host, wake)?;
                }
                Ok(())
            })
            .err();
        return true;
    }
    if let Some(status) = form.monitor.as_ref().and_then(Monitor::take_status) {
        form.status = status;
        return true;
    }
    false
}

pub(super) fn card(s: &State, k: &Kit, p: Palette) -> El {
    let form = &s.hex.shell.speech_host;
    let (status, color) = match (form.recipient.as_ref(), form.monitor.as_ref(), form.status) {
        (None, _, _) => ("Status checks off", p.soft),
        (Some(_), None, _) => ("Status check unavailable", p.warn),
        (_, _, Status::Checking) => ("Checking host", p.soft),
        (_, _, Status::Ready) => ("Speech runtime loaded", p.soft),
        (_, _, Status::Rejected) => ("Token rejected", p.danger),
        (_, _, Status::Unavailable) => ("Host speech runtime unavailable", p.warn),
        (_, _, Status::Amber) => ("Host missed one check", p.warn),
        (_, _, Status::Away) => ("Host away", p.danger),
    };
    let status = match &form.recipient {
        Some(recipient) => format!("{status} ({recipient})"),
        None => status.into(),
    };
    let mut body = col()
        .padding(14.0)
        .gap(8.0)
        .child(roles::SMALL.wrap(status, color).role("status"))
        .child(TextInput::new(&form.url)
            .label("Speech host URL")
            .placeholder("https://host.example")
            .on_edit(|s: &mut State| &mut s.hex.shell.speech_host.url)
            .build(k).key("speech-host-url"))
        .child(TextInput::new(&form.token)
            .secure()
            .label("Device token")
            .on_edit(|s: &mut State| &mut s.hex.shell.speech_host.token)
            .build(k).key("speech-host-token"))
        .child(roles::SMALL.wrap(
            "Allow Sidenote to send the device token to the HTTPS host above for status checks. Meeting audio stays on this device.",
            p.soft,
        ))
        .child(row().gap(8.0).flex_wrap()
            .child(action("Allow status checks", p).key("speech-host-save").on_press(|s: &mut State| {
                if let Some(wake) = s.waker.clone() {
                    let form = &mut s.hex.shell.speech_host;
                    form.error = form.save(wake).err();
                }
            }))
            .child(action("Clear host", p).key("speech-host-clear").on_press(|s: &mut State| {
                if s.waker.is_some() {
                    let form = &mut s.hex.shell.speech_host;
                    form.error = form.clear().err();
                }
            })));
    if let Some(error) = &form.error {
        body = body.child(roles::SMALL.wrap(error, p.warn).role("alert"));
    }
    settings_card("Speech host", body, p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_clears_token_input() {
        let dir = tempfile::tempdir().unwrap();
        let mut form = Form {
            url: TextEdit::new("https://host.example"),
            token: TextEdit::new("synthetic-token"),
            ..Form::default()
        };
        form.token.set_secure(true);
        form.token.focused = true;
        form.save_record(&dir.path().join("host.dpapi")).unwrap();
        assert!(form.token.text.is_empty() && form.token.focused);
    }
}

use super::{State, card as settings_card, roles};
use crate::ui::{theme::Palette, widgets::action};
use rune::{El, TextEdit, Waker, col, row};
use rune_kit::{Kit, TextInput};
use sidenote::core::speech_host::{self, Host, Monitor, Status, WorkMode};

#[derive(Default)]
pub(super) struct Form {
    loaded: bool,
    upload: bool,
    url: TextEdit,
    token: TextEdit,
    recipient: Option<String>,
    monitor: Option<Monitor>,
    status: Status,
    mode: WorkMode,
    progress: String,
    progress_at: Option<std::time::Instant>,
    error: Option<String>,
}

impl Form {
    fn activate(&mut self, host: Host, wake: Waker) -> Result<(), String> {
        self.monitor = None;
        self.recipient = Some(host.recipient().into());
        self.status = Status::Checking;
        self.upload = host.upload_enabled();
        if self.mode != WorkMode::ThisDevice {
            self.monitor = Some(host.monitor(move || wake())?);
        }
        Ok(())
    }

    fn save(&mut self, wake: Waker, upload: bool) -> Result<(), String> {
        let host = self.save_record(&speech_host::settings_path()?, upload)?;
        self.activate(host, wake)
    }

    fn save_record(&mut self, path: &std::path::Path, upload: bool) -> Result<Host, String> {
        let token = self.token.text.trim().as_bytes();
        let host = if upload {
            Host::save_with_upload_consent(path, &self.url.text, token)?
        } else {
            Host::save_and_consent(path, &self.url.text, token)?
        };
        self.url = TextEdit::new(host.recipient());
        self.token.clear();
        Ok(host)
    }

    fn stop_uploads(&mut self, wake: Waker) -> Result<(), String> {
        let path = speech_host::settings_path()?;
        let host = Host::load(&path)?.ok_or("No speech host is saved.")?;
        self.activate(host.stop_audio_uploads(&path)?, wake)
    }

    fn mode(&mut self, mode: WorkMode, wake: Waker) -> Result<(), String> {
        if self.mode == mode {
            return Ok(());
        }
        let path = speech_host::settings_path()?;
        mode.save(&path)
            .map_err(|_| "Could not save the speech work location.")?;
        self.mode = mode;
        self.progress.clear();
        if let Some(host) = Host::load(&path)? {
            self.activate(host, wake)?;
        }
        Ok(())
    }

    fn clear(&mut self) -> Result<(), String> {
        speech_host::clear(&speech_host::settings_path()?, &mut self.monitor)?;
        *self = Self {
            loaded: true,
            mode: self.mode,
            ..Self::default()
        };
        self.token.set_secure(true);
        Ok(())
    }
}

pub(super) fn tick(s: &mut State, speech: bool) -> bool {
    let form = &mut s.hex.shell.speech_host;
    if !form.loaded
        && let Some(wake) = s.waker.clone()
    {
        form.loaded = true;
        form.token.set_secure(true);
        let result = speech_host::settings_path().and_then(|path| {
            form.mode = WorkMode::load(&path)
                .map_err(|_| "Could not load the speech work location.")?
                .0;
            Host::load(&path)
        });
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
    if speech
        && form
            .progress_at
            .is_none_or(|at| at.elapsed().as_secs() >= 1)
    {
        form.progress_at = Some(std::time::Instant::now());
        let progress = speech_host::settings_path()
            .map(|path| speech_host::progress(&path))
            .unwrap_or_default();
        if progress != form.progress {
            form.progress = progress;
            return true;
        }
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
    let mut work = row().gap(8.0).flex_wrap();
    for (mode, label) in [
        (WorkMode::Auto, "Auto"),
        (WorkMode::HostOnly, "Host only"),
        (WorkMode::ThisDevice, "This device"),
    ] {
        let label = if form.mode == mode {
            format!("{label} (selected)")
        } else {
            label.into()
        };
        work = work.child(action(&label, p).on_press(move |s: &mut State| {
            if let Some(wake) = s.waker.clone() {
                let form = &mut s.hex.shell.speech_host;
                form.error = form.mode(mode, wake).err();
            }
        }));
    }
    let mut body = col()
        .padding(14.0)
        .gap(8.0)
        .child(roles::SMALL.wrap("Do the work on", p.soft))
        .child(work)
        .child(roles::SMALL.wrap(
            if form.mode == WorkMode::ThisDevice { "Speech runs on this device".into() } else { status },
            color,
        ).role("status"))
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
            if form.mode == WorkMode::ThisDevice {
                "This device sends no audio or status checks to the speech host."
            } else if form.upload {
                "Audio uploads are on for the saved host. Sidenote sends audio windows from new meetings and the device token when the host is ready."
            } else {
                "Allow audio uploads to send audio windows from new meetings and the device token to the HTTPS host above when it is ready. Status checks start when Sidenote opens."
            },
            p.soft,
        ))
        .child(row().gap(8.0).flex_wrap()
            .child(action("Allow status checks", p).key("speech-host-save").on_press(|s: &mut State| {
                if let Some(wake) = s.waker.clone() {
                    let form = &mut s.hex.shell.speech_host;
                    form.error = form.save(wake, false).err();
                }
            }))
            .child(action(
                if form.upload { "Stop audio uploads" } else { "Allow audio uploads" }, p
            ).key("speech-host-upload").on_press(|s: &mut State| {
                if let Some(wake) = s.waker.clone() {
                    let form = &mut s.hex.shell.speech_host;
                    form.error = if form.upload { form.stop_uploads(wake) } else { form.save(wake, true) }.err();
                }
            }))
            .child(action("Clear host", p).key("speech-host-clear").on_press(|s: &mut State| {
                if s.waker.is_some() {
                    let form = &mut s.hex.shell.speech_host;
                    form.error = form.clear().err();
                }
            })));
    if !form.progress.is_empty() {
        body = body.child(roles::SMALL.wrap(&form.progress, p.soft).role("status"));
    }
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
        form.save_record(&dir.path().join("host.dpapi"), false)
            .unwrap();
        assert!(form.token.text.is_empty() && form.token.focused);
    }
}

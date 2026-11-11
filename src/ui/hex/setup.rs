use super::*;
use crate::ui::widgets::{ButtonRole, button};
use sidenote::hex_install::{self, Installation, State as InstallState};

impl State {
    pub fn hex_needs_install(&self) -> bool {
        self.hex.installation == Installation::Missing
    }

    fn install_hex(&mut self) {
        if super::super::isolated() || self.hex.running() {
            return;
        }
        self.hex.installation = hex_install::detect();
        if !self.hex.install.begin(&self.hex.installation) {
            return;
        }
        let events = self.events_tx.clone();
        let waker = self.waker.clone();
        if thread::Builder::new()
            .name("hex-install".into())
            .spawn(move || {
                hex_install::install_after_click(|state| {
                    let _ = events.send(Event::Hex(Msg::Install(state)));
                    if let Some(waker) = &waker {
                        waker();
                    }
                });
            })
            .is_err()
        {
            self.hex.install = InstallState::Failed("Could not start Hex setup. Try again.");
        }
    }

    fn open_installed_hex(&mut self) {
        if super::super::isolated() {
            return;
        }
        self.hex.installation = hex_install::detect();
        if let Installation::Installed { exe, .. } = &self.hex.installation {
            use std::{
                os::windows::process::CommandExt,
                process::{Command, Stdio},
            };
            if Command::new(exe)
                .arg("app")
                .creation_flags(0x08000000)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .is_err()
            {
                self.hex.install =
                    InstallState::Failed("Could not open Hex. Open it from the Start menu.");
            }
        }
    }

    fn hex_download_page(&mut self) {
        if super::super::isolated() {
            return;
        }
        use windows_sys::Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL};
        let url: Vec<u16> = hex_install::DOWNLOAD_PAGE
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
        // SAFETY: both strings are NUL-terminated and live through this call; URL is constant.
        let result = unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                verb.as_ptr(),
                url.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            )
        };
        if result as isize <= 32 {
            self.hex.install = InstallState::Failed(
                "Could not open the download page. Visit github.com/anomalyco/hex/releases/latest.",
            );
        }
    }
}

pub(super) fn card(s: &State, p: Palette) -> Option<El> {
    let h = &s.hex;
    if h.running() && !h.install.busy() {
        return None;
    }
    let (title, message) = match (&h.install, &h.installation) {
        (InstallState::Idle, Installation::Installed { version, .. }) => (
            "Open Hex", format!("Hex {version} is installed. Open Hex to record and transcribe."),
        ),
        (InstallState::Idle, Installation::Incomplete) => (
            "Check the Hex installation", "Sidenote found Hex files but could not read a working installation. Use the download page to repair it, then check again.".into(),
        ),
        (InstallState::Complete, _) => ("Hex is installed", h.install.message()),
        (InstallState::Failed(_), _) => ("Hex setup needs attention", h.install.message()),
        _ => ("Install Hex", h.install.message()),
    };
    let mut actions = row().w_full().flex_wrap().gap(8.0);
    if h.installation == Installation::Missing && !h.running() {
        actions = actions.child(
            button(
                if matches!(h.install, InstallState::Failed(_)) {
                    "Try again"
                } else {
                    "Install Hex"
                },
                "hex-install",
                ButtonRole::Primary,
                p,
            )
            .disabled(h.install.busy())
            .on_press(|s: &mut State| s.install_hex()),
        );
    } else if matches!(h.installation, Installation::Installed { .. }) && !h.running() {
        actions = actions.child(
            button("Open Hex", "hex-setup-open", ButtonRole::Primary, p)
                .disabled(h.install.busy())
                .on_press(|s: &mut State| s.open_installed_hex()),
        );
    }
    actions = actions
        .child(
            button(
                "Download page",
                "hex-download-page",
                ButtonRole::Secondary,
                p,
            )
            .on_press(|s: &mut State| s.hex_download_page()),
        )
        .child(
            button("Check again", "hex-install-recheck", ButtonRole::Ghost, p)
                .disabled(h.install.busy())
                .on_press(|s: &mut State| {
                    if super::super::isolated() {
                        return;
                    }
                    s.hex.installation = hex_install::detect();
                    s.hex.install = InstallState::Idle;
                    s.hex_refresh_if_due();
                }),
        );
    Some(col().key("hex-setup").w_full().px(20.0).py(20.0).mt(16.0).mb(16.0).gap(12.0)
        .fill(p.card).border(1.0, p.line).radius(10.0)
        .child(roles::SECTION.wrap(title, p.ink))
        .child(roles::SMALL.wrap(message, if matches!(h.install, InstallState::Failed(_)) { p.warn } else { p.soft })
            .key("hex-install-progress").role(if matches!(h.install, InstallState::Failed(_)) { "alert" } else { "status" }))
        .children((h.install == InstallState::Idle && h.installation == Installation::Missing).then(||
            roles::META.wrap("Downloads the latest stable Windows installer from Hex on GitHub. Checks its SHA-256 and publisher before installing. No administrator access. Hex may download a speech model when you open it.", p.faint)))
        .child(actions))
}

use super::{Event, State, area, roles, tasks::button, theme::Palette};
use rune::{El, col};
use sidenote::core::mcp_connect::{self, Client};

#[derive(Default)]
pub struct Connection {
    pub client: Client,
    pub busy: bool,
    pub show_details: bool,
    pub show_all: bool,
    pub result: Option<Result<String, String>>,
}
impl State {
    fn connect_claude(&mut self) {
        if self.connection.busy {
            return;
        }
        self.connection.busy = true;
        self.connection.result = None;
        let events = self.events_tx.clone();
        let wake = self.waker.clone();
        std::thread::spawn(move || {
            let result = std::env::current_exe()
                .map_err(|_| "Could not find Sidenote's executable.".to_string())
                .and_then(|exe| mcp_connect::add_to_claude(&exe));
            let _ = events.send(Event::McpConnected(result));
            if let Some(wake) = wake {
                wake();
            }
        });
    }
}
pub fn section(s: &State, p: Palette) -> El {
    let mut body = col().w_full().gap(8.0)
        .child(roles::SMALL.wrap("Connected apps can read your meetings, typed notes, tasks and memory through this connection, and send what they read to their AI provider.",p.soft));
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(_) => {
            return body.child(roles::SMALL.wrap(
                "Could not find this executable. Reopen Sidenote and try again.",
                p.warn,
            ));
        }
    };
    let mut clients = col().w_full();
    for client in Client::ALL {
        let found = client.config_path().is_file()
            || (client == Client::ClaudeCode
                && s.ai_notes
                    .found
                    .contains(&(sidenote::core::notes::Helper::Claude, true)));
        if !found && !s.connection.show_all {
            continue;
        }
        clients = clients.child(super::hex::settings::setting_row(
            s,
            &format!("connect-{}", client.name()),
            client.name(),
            "It can read your meetings through Sidenote.",
            button(
                "Set up",
                format!("connect-{}-setup", client.name()),
                false,
                p,
            )
            .on_press(move |s: &mut State| {
                s.connection.client = client;
                s.connection.show_details = true;
            }),
            false,
            p,
        ));
    }
    body = body.child(clients).child(
        button(
            if s.connection.show_all {
                "Hide other apps"
            } else {
                "Connect another app"
            },
            "connect-more",
            false,
            p,
        )
        .on_press(|s: &mut State| s.connection.show_all = !s.connection.show_all),
    );
    if !s.connection.show_details {
        return body;
    }
    body = body.child(roles::META.wrap("Feature switches change what Sidenote shows. They do not limit what connected apps can read through meetings.",p.soft));
    let client = s.connection.client;
    let config = client.config(&exe);
    body = body.child(roles::NAME.wrap(client.name(), p.ink));
    if client == Client::ClaudeCode {
        let mut add = button(
            if s.connection.busy {
                "Adding..."
            } else {
                "Add to Claude Code"
            },
            "connect-claude-add",
            true,
            p,
        );
        if !s.connection.busy {
            add = add.on_press(|s: &mut State| s.connect_claude());
        }
        body = body.child(add)
            .child(roles::META.wrap("Adds a user connection using the installed Claude Code CLI. An existing sidenote connection is kept.", p.soft));
        if let Some(result) = &s.connection.result {
            body = body.child(
                roles::SMALL
                    .wrap(
                        match result {
                            Ok(text) | Err(text) => text.clone(),
                        },
                        if result.is_ok() { p.ink } else { p.warn },
                    )
                    .role("status"),
            );
        }
    }
    body.child(roles::META.wrap(format!("Config file: {}", client.config_path().display()), p.soft))
        .child(roles::SMALL.wrap(if client == Client::ClaudeCode { "Or run this command in PowerShell:" } else { "Merge this into your config, keeping the other entries:" }, p.soft))
        .child(col().w_full().px(12.0).py(12.0).radius(8.0).fill(p.chip).child(roles::SMALL.wrap(config.clone(), p.ink)))
        .child(area::button(if client == Client::ClaudeCode { "Copy command" } else { "Copy config" }, "connect-copy", p).on_press_tx(move |s: &mut State, tx| { tx.copy(config.clone()); s.show_toast("Copied", tx); }))
        .child(roles::META.wrap(if client == Client::ClaudeCode { "Restart its MCP session after setup. The button uses Claude Code to manage its own config." } else { "Restart or reconnect your AI app after setup. Sidenote only copies this text; it does not edit your config." }, p.soft))
}

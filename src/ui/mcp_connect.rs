use super::{Event, State, area, roles, tasks::button, theme::Palette, widgets::hairline};
use rune::{El, col, row};
use sidenote::core::mcp_connect::{self, Client};

#[derive(Default)]
pub struct Connection {
    pub client: Client,
    pub busy: bool,
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
    let mut body = col().w_full().pt(24.0).gap(12.0)
        .child(roles::SECTION.wrap("Connect AI apps", p.ink))
        .child(roles::SMALL.wrap("Use your meeting knowledge in an AI app on this PC.", p.soft))
        .child(roles::SMALL.wrap("Connected apps can read your meetings, typed notes, tasks and memory through this connection, and send what they read to their AI provider.", p.soft))
        .child(roles::META.wrap("Feature switches change what Sidenote shows. They do not limit what connected apps can read through meetings.", p.soft));
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(_) => {
            return body.child(roles::SMALL.wrap(
                "Could not find this executable. Reopen Sidenote and try again.",
                p.warn,
            ));
        }
    };
    body = body
        .child(hairline(p))
        .child(roles::NAME.wrap(
            if sidenote::updates::installed() {
                "Installed executable"
            } else {
                "Development build executable"
            },
            p.ink,
        ))
        .child(roles::SMALL.wrap(exe.to_string_lossy(), p.soft));
    if !sidenote::updates::installed() {
        body = body.child(roles::META.wrap("This is a dev build. Keep this file here, or reconnect using your installed copy after installing Sidenote.", p.soft));
    }
    let mut clients = row().w_full().flex_wrap().gaps(8.0, 8.0);
    for client in Client::ALL {
        clients = clients.child(
            button(
                client.name(),
                format!("connect-{}", client.name()),
                s.connection.client == client,
                p,
            )
            .on_press(move |s: &mut State| s.connection.client = client),
        );
    }
    let client = s.connection.client;
    let config = client.config(&exe);
    body = body
        .child(clients)
        .child(roles::NAME.wrap(client.name(), p.ink));
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

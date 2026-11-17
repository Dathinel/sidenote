use super::widgets::ButtonDisabled;
use super::{Event, State, area, roles, tasks::button, theme::Palette};
use rune::{El, col};
use sidenote::core::mcp_connect::{self, Agent, Client, Status};

#[derive(Default)]
pub struct Connection {
    pub client: Client,
    pub busy: bool,
    pub show_details: bool,
    pub show_all: bool,
    pub agents: Vec<Agent>,
    pub results: Vec<(Client, Result<String, String>)>,
}
pub struct Update {
    pub agents: Vec<Agent>,
    pub results: Vec<(Client, Result<String, String>)>,
}
impl State {
    pub(crate) fn connect_agents(&mut self, add: bool, client: Option<Client>) {
        if self.connection.busy || super::isolated() {
            return;
        }
        self.connection.busy = true;
        if add {
            self.connection.results.clear();
        }
        let events = self.events_tx.clone();
        let wake = self.waker.clone();
        std::thread::spawn(move || {
            let agents = mcp_connect::discover();
            let selected: Vec<_> = agents
                .into_iter()
                .filter(|a| client.is_none_or(|c| c == a.client))
                .collect();
            let results = if add {
                match std::env::current_exe() {
                    Ok(exe) => mcp_connect::add_all(&selected, &exe),
                    Err(_) => selected
                        .iter()
                        .map(|a| {
                            (
                                a.client,
                                Err("Could not find Sidenote's executable.".into()),
                            )
                        })
                        .collect(),
                }
            } else {
                vec![]
            };
            let _ = events.send(Event::McpConnected(Update {
                agents: mcp_connect::discover(),
                results,
            }));
            if let Some(wake) = wake {
                wake();
            }
        });
    }
}
pub fn section(s: &State, p: Palette) -> El {
    let mut body = col().w_full().gap(8.0)
        .child(roles::SMALL.wrap("Connected apps can read your meetings, typed notes, tasks and memory through this connection, and send what they read to their AI provider.",p.soft).padding(14.0));
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(_) => {
            return body.child(roles::SMALL.wrap(
                "Could not find this executable. Reopen Sidenote and try again.",
                p.warn,
            ));
        }
    };
    let busy = s.connection.busy;
    let mut agents: Vec<_> = s
        .connection
        .agents
        .iter()
        .filter(|a| a.client.visible(cfg!(target_os = "linux")))
        .collect();
    agents.sort_by_key(|a| a.status == Status::NotInstalled);
    for agent in agents {
        let client = agent.client;
        if agent.status == Status::NotInstalled && !s.connection.show_all {
            continue;
        }
        let label = match agent.status {
            Status::Added => "Added ✓",
            Status::NotAdded => "Add",
            Status::NotInstalled => "Not installed",
        };
        let mut action = button(label, format!("connect-{}-add", client.name()), false, p)
            .button_disabled(busy || agent.status != Status::NotAdded);
        if !busy && agent.status == Status::NotAdded {
            action = action.on_press(move |s: &mut State| {
                s.connection.client = client;
                s.connect_agents(true, Some(client));
            });
        }
        body = body.child(
            super::hex::settings::setting_row(
                s,
                &format!("connect-{}", client.name()),
                client.name(),
                &format!("{} · Select for config", agent.status.text()),
                action,
                false,
                p,
            )
            .on_press(move |s: &mut State| {
                s.connection.client = client;
                s.connection.show_details = true;
            }),
        );
        if let Some((_, result)) = s.connection.results.iter().find(|(c, _)| *c == client) {
            let (Ok(text) | Err(text)) = result;
            body = body.child(
                roles::SMALL
                    .wrap(text.clone(), if result.is_ok() { p.ink } else { p.warn })
                    .role("status"),
            );
        } else if let Some(detail) = &agent.detail {
            body = body.child(roles::SMALL.wrap(detail.clone(), p.warn).role("status"));
        }
        body = body.child(super::widgets::hairline(p));
    }
    body = body
        .child(
            button(
                if busy {
                    "Checking..."
                } else {
                    "Add to all installed"
                },
                "connect-all",
                true,
                p,
            )
            .button_disabled(
                busy || !s
                    .connection
                    .agents
                    .iter()
                    .any(|a| a.status == Status::NotAdded),
            )
            .on_press(|s: &mut State| s.connect_agents(true, None)),
        )
        .child(
            button("Recheck", "connect-recheck", false, p)
                .button_disabled(busy)
                .on_press(|s: &mut State| s.connect_agents(false, None)),
        )
        .child(
            button(
                if s.connection.show_all {
                    "Hide other apps"
                } else {
                    "Show all"
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
    let Some(agent) = s.connection.agents.iter().find(|a| a.client == client) else {
        return body;
    };
    let config = client.config(&exe);
    body = body.child(roles::NAME.wrap(client.name(), p.ink));
    body.child(roles::META.wrap(format!("Config file: {}", agent.path.display()), p.soft))
        .child(roles::SMALL.wrap(if matches!(client, Client::ClaudeCode) { "Or run this command in PowerShell:" } else { "Merge this into your config, keeping the other entries:" }, p.soft))
        .child(col().w_full().px(12.0).py(12.0).radius(8.0).fill(p.chip).child(roles::SMALL.wrap(config.clone(), p.ink)))
        .child(area::button(if matches!(client, Client::ClaudeCode) { "Copy command" } else { "Copy config" }, "connect-copy", p).on_press_tx(move |s: &mut State, tx| { tx.copy(config.clone()); s.show_toast("Copied", tx); }))
        .child(roles::META.wrap(if matches!(client, Client::ClaudeCode) { "Restart its MCP session after setup. The button uses Claude Code to manage its own config." } else { "Restart or reconnect your AI app after setup. The Add button keeps existing entries and verifies the saved connection." }, p.soft))
}

/// Render fixtures never inspect or write the owner's client configurations.
pub fn scene(s: &mut State, name: &str) {
    assert!(super::isolated());
    s.connection.show_all = name == "settings-mcp-none";
    s.config.features.memory = false;
    s.connection.agents = Client::ALL
        .into_iter()
        .enumerate()
        .map(|(i, client)| Agent {
            client,
            path: s
                .store
                .root
                .join("fake-agents")
                .join(client.name())
                .join("config.json"),
            cli: None,
            status: if name == "settings-mcp-added" {
                Status::Added
            } else if name == "settings-mcp-none" || i == 4 {
                Status::NotInstalled
            } else if i % 2 == 0 {
                Status::Added
            } else {
                Status::NotAdded
            },
            detail: None,
        })
        .collect();
}

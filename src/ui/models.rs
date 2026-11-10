use super::{
    Event, State, hex, roles,
    theme::Palette,
    widgets::{link, pressable},
};
use rune::{El, col, row};
use rune_kit::{Kit, TextInput};
use sidenote::core::model_catalog::{self, ModelStore, Phase, Progress, Runtime, SizeTier};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

pub enum Msg {
    Progress(&'static str, Progress),
    Finished(&'static str, std::io::Result<()>),
}
struct Job {
    cancel: Arc<AtomicBool>,
    worker: JoinHandle<()>,
    removing: bool,
}

#[derive(Default)]
pub struct Models {
    languages: Vec<String>,
    installed: HashSet<&'static str>,
    partial: HashSet<&'static str>,
    progress: HashMap<&'static str, Progress>,
    errors: HashMap<&'static str, String>,
    jobs: HashMap<&'static str, Job>,
    preference_error: Option<String>,
    scene: bool,
}
const LANGUAGES: &[(&str, &str)] = &[
    ("en", "English"),
    ("hi", "Hindi"),
    ("bn", "Bengali"),
    ("ta", "Tamil"),
    ("te", "Telugu"),
    ("mr", "Marathi"),
];
impl Models {
    pub fn load(root: &Path) -> Self {
        let (languages, preference_error) = match model_catalog::load_meeting_languages(root) {
            Ok(languages) => (languages, None),
            Err(e) => (
                vec!["en".into()],
                Some(format!("Could not read meeting languages: {e}")),
            ),
        };
        let store = ModelStore::new(root.to_owned());
        let installed: HashSet<_> = model_catalog::catalog()
            .iter()
            .filter(|m| store.installed_paths(m.id).ok().flatten().is_some())
            .map(|m| m.id)
            .collect();
        let partial = model_catalog::catalog()
            .iter()
            .filter(|m| !installed.contains(m.id) && store.stored_bytes(m.id).unwrap_or(0) > 0)
            .map(|m| m.id)
            .collect();
        Self {
            languages,
            installed,
            partial,
            preference_error,
            progress: HashMap::new(),
            errors: HashMap::new(),
            jobs: HashMap::new(),
            scene: false,
        }
    }
    pub fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Progress(id, progress) => {
                self.progress.insert(id, progress);
            }
            Msg::Finished(id, result) => {
                if let Some(job) = self.jobs.remove(id) {
                    let _ = job.worker.join();
                }
                self.progress.remove(id);
                let store = ModelStore::default();
                if store.installed_paths(id).ok().flatten().is_some() {
                    self.installed.insert(id);
                } else {
                    self.installed.remove(id);
                }
                if !self.installed.contains(id) && store.stored_bytes(id).unwrap_or(0) > 0 {
                    self.partial.insert(id);
                } else {
                    self.partial.remove(id);
                }
                match result {
                    Ok(()) => {
                        self.errors.remove(id);
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                        self.errors.remove(id);
                    }
                    Err(error) => {
                        self.errors.insert(id, error.to_string());
                    }
                }
            }
        }
    }
}
impl Drop for Models {
    fn drop(&mut self) {
        for job in self.jobs.values() {
            job.cancel.store(true, Ordering::Relaxed);
        }
        for (_, job) in self.jobs.drain() {
            let _ = job.worker.join();
        }
    }
}
impl State {
    fn model_language(&mut self, language: &'static str) {
        let mut languages = self.models.languages.clone();
        if languages.iter().any(|l| l == language) {
            languages.retain(|l| l != language);
        } else {
            languages.push(language.into());
        }
        if languages.is_empty() {
            self.models.preference_error = Some("Choose at least one spoken language.".into());
            return;
        }
        if !self.models.scene
            && let Err(e) = model_catalog::save_meeting_languages(&self.store.root, &languages)
        {
            self.models.preference_error = Some(format!("Could not save meeting languages: {e}"));
            return;
        }
        self.models.languages = languages;
        self.models.preference_error = None;
        self.hex.shell.saved = Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
    }
    fn model_download(&mut self, id: &'static str, remove: bool) {
        if self.models.scene || self.models.jobs.contains_key(id) {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let sender = self.events_tx.clone();
        let waker = self.waker.clone();
        self.models.errors.remove(id);
        self.models.progress.insert(
            id,
            Progress {
                downloaded: 0,
                total: model_catalog::model(id).map_or(0, |m| m.bytes()),
                phase: Phase::Downloading,
            },
        );
        let worker = thread::spawn(move || {
            let result = if remove {
                ModelStore::default().remove(id)
            } else {
                ModelStore::default().install(id, &worker_cancel, |p| {
                    let _ = sender.send(Event::Models(Msg::Progress(id, p)));
                    if let Some(wake) = &waker {
                        wake();
                    }
                })
            };
            let _ = sender.send(Event::Models(Msg::Finished(id, result)));
            if let Some(wake) = &waker {
                wake();
            }
        });
        self.models.jobs.insert(
            id,
            Job {
                cancel,
                worker,
                removing: remove,
            },
        );
    }
}
pub fn language_picker(s: &State, k: &Kit, p: Palette) -> El {
    let names = LANGUAGES
        .iter()
        .filter(|(id, _)| s.models.languages.iter().any(|language| language == id))
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = col()
        .key("spoken-languages")
        .fill(super::hex::settings::highlight(s, "spoken-languages", p))
        .w_full()
        .padding(14.0)
        .gap(8.0)
        .child(super::hex::settings::setting_row(
            s,
            "languages-row",
            "Languages you speak",
            "Choose the languages people speak in your meetings.",
            pressable(roles::CONTROL.wrap(names, p.ink))
                .key("languages-dropdown")
                .label("Languages you speak")
                .on_press(|s: &mut State| {
                    if !s.hex.open_language_picker() {
                        s.hex.close_language_picker();
                    }
                }),
            true,
            p,
        ));
    if s.hex.language_picker_open() {
        out = out.child(language_column(s, k, p));
    }
    out
}
fn language_column(s: &State, k: &Kit, p: Palette) -> El {
    let mut languages = col().w_full().gap(8.0).child(
        TextInput::new(&s.hex.shell.language_query)
            .placeholder("Search languages")
            .label("Search languages")
            .height(34.0)
            .on_edit(|s: &mut State| &mut s.hex.shell.language_query)
            .build(k)
            .key("language-search")
            .w_full(),
    );
    for &(id, name) in LANGUAGES {
        if !name.to_lowercase().contains(
            s.hex
                .shell
                .language_query
                .text
                .trim()
                .to_lowercase()
                .as_str(),
        ) {
            continue;
        }
        let selected = s.models.languages.iter().any(|l| l == id);
        languages = languages.child(
            pressable(
                row()
                    .w_full()
                    .gap(8.0)
                    .items_center()
                    .child(
                        roles::CONTROL
                            .line(if selected { "\u{2611}" } else { "\u{2610}" }, p.accent),
                    )
                    .child(roles::CONTROL.line(name, p.ink)),
            )
            .key(format!("meeting-language-{id}"))
            .role("checkbox")
            .checked(selected)
            .label(name)
            .on_press(move |s: &mut State| s.model_language(id)),
        );
    }
    languages
}
pub fn view(s: &State, p: Palette, routing: El) -> El {
    if !cfg!(feature = "asr-runtime") {
        return super::hex::settings::setting_row(
            s,
            "speech-models",
            "Transcribe with: Hex",
            "Uses the speech model managed by Hex.",
            roles::CONTROL.line("In use", p.ok),
            false,
            p,
        );
    }
    let k = super::theme::kit(&p);
    row()
        .w_full()
        .flex_wrap()
        .gap(18.0)
        .mt(16.0)
        .child(
            col()
                .w(160.0)
                .gap(10.0)
                .child(roles::GROUP.wrap("Meeting languages", p.soft))
                .child(language_column(s, &k, p)),
        )
        .child(catalog_view(s, p, routing, false).grow(1.0).min_w(240.0))
}
pub fn advanced(s: &State, p: Palette) -> El {
    catalog_view(s, p, rune::el(), true)
}
fn catalog_view(s: &State, p: Palette, routing: El, advanced: bool) -> El {
    let k = super::theme::kit(&p);
    let models = &s.models;
    let mut body = col().w_full().gap(12.0).mt(16.0);
    if !advanced {
        body = body
            .child(
                row()
                    .w_full()
                    .flex_wrap()
                    .gap(12.0)
                    .child(
                        TextInput::new(&s.hex.shell.model_query)
                            .placeholder("Search speech models")
                            .label("Search speech models")
                            .height(34.0)
                            .on_edit(|s: &mut State| &mut s.hex.shell.model_query)
                            .build(&k)
                            .key("model-search")
                            .grow(1.0)
                            .min_w(140.0),
                    )
                    .child(
                        pressable(roles::CONTROL.line(
                            if s.hex.shell.model_recommended {
                                "Recommended \u{2713}"
                            } else {
                                "Recommended"
                            },
                            p.soft,
                        ))
                        .key("model-recommended")
                        .on_press(|s: &mut State| {
                            s.hex.shell.model_recommended = !s.hex.shell.model_recommended
                        }),
                    ),
            )
            .child(routing);
    }
    if let Some(e) = &models.preference_error {
        body = body.child(roles::SMALL.medium().wrap(e, p.warn).role("alert"));
    }
    // English is Hex's Parakeet: nothing to download. Speech models show for the other chosen
    // languages; language detection shows once a second language is chosen (ADR 0009 (b)).
    let others: Vec<&str> = models
        .languages
        .iter()
        .map(String::as_str)
        .filter(|l| *l != "en")
        .collect();
    let visible: Vec<_> = model_catalog::catalog()
        .iter()
        .filter(|m| {
            if advanced {
                return m.role == model_catalog::Role::LanguageId || !cfg!(feature = "asr-runtime");
            }
            if m.role == model_catalog::Role::LanguageId {
                return false;
            }
            if !m
                .name
                .to_lowercase()
                .contains(s.hex.shell.model_query.text.trim().to_lowercase().as_str())
            {
                return false;
            }
            if s.hex.shell.model_recommended
                && m.runtime != Runtime::Hex
                && !model_catalog::PROVISIONAL_DEFAULT_IDS.contains(&m.id)
            {
                return false;
            }
            if m.runtime == Runtime::Hex {
                return true;
            }
            // Downloaded models stay listed so they can be removed after a language is dropped.
            if models.installed.contains(m.id) || models.progress.contains_key(m.id) {
                return true;
            }
            if m.role == model_catalog::Role::LanguageId {
                models.languages.len() > 1
            } else {
                m.languages.iter().any(|l| others.contains(l))
            }
        })
        .collect();
    if visible.is_empty() {
        return body.child(roles::SMALL.wrap(
            "No speech models match this search and the selected languages.",
            p.soft,
        ));
    }
    for m in visible {
        let id = m.id;
        let info = col()
            .grow(1.0)
            .min_w(160.0)
            .gap(5.0)
            .child(roles::NAME.wrap(m.name, p.ink))
            .child(roles::SMALL.wrap(
                if m.runtime == Runtime::Hex {
                    "Uses the copy already in Hex. Nothing to download."
                } else if m.role == model_catalog::Role::LanguageId {
                    "Not available yet."
                } else {
                    "Transcribes the meeting languages listed below."
                },
                p.soft,
            ))
            .child(model_facts(m, p));
        let mut card = col().gap(6.0).max_w(230.0);
        if m.runtime == Runtime::Hex {
            card = card.child(roles::CONTROL.line("In Hex. In use", p.ok));
        } else {
            if m.role == model_catalog::Role::LanguageId {
                card = card.child(roles::SMALL.wrap(
                    "Not available yet. The publisher's patched runtime is not bundled.",
                    p.soft,
                ));
                if models.installed.contains(id) || models.partial.contains(id) {
                    card = card.child(
                        link("Remove", p)
                            .key(format!("model-remove-{id}"))
                            .on_press(move |s: &mut State| s.model_download(id, true)),
                    );
                }
                body = body.child(
                    row()
                        .key(format!("speech-model-{id}"))
                        .w_full()
                        .flex_wrap()
                        .items_center()
                        .gaps(14.0, 10.0)
                        .padding(14.0)
                        .radius(8.0)
                        .border(1.0, p.line)
                        .fill(p.card)
                        .child(info)
                        .child(card),
                );
                continue;
            }
            if let Some(progress) = models.progress.get(id) {
                if models.jobs.get(id).is_some_and(|job| job.removing) {
                    card = card.child(
                        roles::CONTROL
                            .wrap("Removing downloaded files...", p.soft)
                            .role("status"),
                    );
                } else {
                    let cancelling = models
                        .jobs
                        .get(id)
                        .is_some_and(|job| job.cancel.load(Ordering::Relaxed));
                    let phase = if cancelling {
                        "Cancelling"
                    } else if matches!(progress.phase, Phase::Verifying) {
                        "Verifying checksum"
                    } else {
                        "Downloading"
                    };
                    card = card
                        .child(
                            roles::CONTROL
                                .wrap(
                                    format!(
                                        "{phase}: {:.1} / {:.1} MB",
                                        progress.downloaded as f64 / 1_000_000.0,
                                        progress.total as f64 / 1_000_000.0
                                    ),
                                    p.accent,
                                )
                                .role("status"),
                        )
                        .child(
                            link("Cancel download", p)
                                .key(format!("model-cancel-{id}"))
                                .on_press(move |s: &mut State| {
                                    if let Some(job) = s.models.jobs.get(id) {
                                        job.cancel.store(true, Ordering::Relaxed);
                                    }
                                }),
                        );
                }
            } else if models.installed.contains(id) {
                card = card.child(
                    row()
                        .gap(14.0)
                        .child(roles::CONTROL.wrap("Installed", p.accent))
                        .child(
                            link("Remove", p)
                                .key(format!("model-remove-{id}"))
                                .label(format!("Remove {}", m.name))
                                .on_press(move |s: &mut State| s.model_download(id, true)),
                        ),
                );
            } else {
                card = card.child(
                    link(
                        if models.partial.contains(id) {
                            "Resume download"
                        } else {
                            "Download"
                        },
                        p,
                    )
                    .key(format!("model-download-{id}"))
                    .label(format!(
                        "Download {}, {:.1} MB, {}",
                        m.name,
                        m.bytes() as f64 / 1_000_000.0,
                        m.licence
                    ))
                    .on_press(move |s: &mut State| s.model_download(id, false)),
                );
            }
            if models.partial.contains(id)
                && !models.installed.contains(id)
                && !models.progress.contains_key(id)
            {
                card = card.child(roles::meta("Partial download saved on this device.", p));
                card = card.child(
                    link("Remove partial download", p)
                        .key(format!("model-clear-partial-{id}"))
                        .on_press(move |s: &mut State| s.model_download(id, true)),
                );
            }
            if let Some(error) = models.errors.get(id) {
                card = card.child(roles::SMALL.medium().wrap(error, p.warn).role("alert"));
            }
        }
        body = body.child(
            row()
                .key(format!("speech-model-{id}"))
                .w_full()
                .flex_wrap()
                .items_center()
                .gaps(14.0, 10.0)
                .padding(14.0)
                .radius(8.0)
                .border(1.0, p.line)
                .fill(p.card)
                .child(info)
                .child(card),
        );
    }
    body.child(roles::meta("Downloads stay on this device. Memory figures cover the model process. Your Hex dictation model stays as you chose it.", p).mt(roles::GROUP_TOP))
}

/// A model's facts on one quiet line: languages, licence, tier, download size and memory.
fn model_facts(m: &model_catalog::Model, p: Palette) -> El {
    let langs = if m.languages.len() > 4 {
        format!("{} languages", m.languages.len())
    } else {
        m.languages
            .iter()
            .map(|l| {
                LANGUAGES
                    .iter()
                    .find(|(id, _)| id == l)
                    .map_or(*l, |(_, name)| *name)
            })
            .collect::<Vec<_>>()
            .join(", ")
    };
    let tier = if m.runtime == Runtime::Hex {
        "Managed by Hex"
    } else if m.size_tier() == SizeTier::Default {
        "Recommended"
    } else {
        "Optional"
    };
    let mut facts = vec![langs, format!("{} licence", m.licence), tier.to_string()];
    if m.runtime != Runtime::Hex {
        facts.push(format!("{:.1} MB download", m.bytes() as f64 / 1_000_000.0));
        if let Some(memory) = m.minimum_ram_mb {
            facts.push(format!("Measured model memory {memory} MB"));
        }
    }
    roles::meta_line(&facts, p)
}
pub fn scene(s: &mut State, name: &str) {
    hex::scene(s, false);
    s.hex_open = true;
    s.hex.section = hex::Section::MeetingModels;
    s.models = Models {
        languages: vec!["en".into(), "hi".into()],
        scene: true,
        installed: HashSet::new(),
        partial: HashSet::new(),
        progress: HashMap::new(),
        errors: HashMap::new(),
        jobs: HashMap::new(),
        preference_error: None,
    };
    let id = "indicconformer-hi-ctc";
    match name {
        "downloading" => {
            s.models.progress.insert(
                id,
                Progress {
                    downloaded: 73_000_000,
                    total: model_catalog::model(id).unwrap().bytes(),
                    phase: Phase::Downloading,
                },
            );
        }
        "installed" => {
            s.models.installed.insert(id);
        }
        "checksum-error" => {
            s.models.errors.insert(
                id,
                "Checksum verification failed. The downloaded file was deleted. Try again.".into(),
            );
        }
        _ => {}
    }
}

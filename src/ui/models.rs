use super::{
    Event, State, hex, roles,
    theme::Palette,
    widgets::{hairline, label, link, pressable},
};
use rune::{El, col, row};
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
pub fn view(s: &State, p: Palette, routing: El) -> El {
    let models = &s.models;
    let mut body = col()
        .w_full()
        .gap(12.0)
        .child(
            roles::SECTION
                .wrap("Meeting languages and models", p.ink)
                .mt(10.0),
        )
        .child(roles::SMALL.wrap(
            "Choose the languages people speak in your meetings.",
            p.soft,
        ));
    let mut languages = row().w_full().flex_wrap().gap(8.0);
    for &(id, name) in LANGUAGES {
        let selected = models.languages.iter().any(|l| l == id);
        languages = languages.child(
            pressable(
                label(
                    name,
                    13.0,
                    if selected { 600.0 } else { 500.0 },
                    if selected { p.accent } else { p.soft },
                )
                .px(10.0)
                .py(5.0)
                .radius(8.0)
                .fill(if selected {
                    p.accent.alpha(0.14)
                } else {
                    p.accent.alpha(0.0)
                })
                .border(1.0, if selected { p.accent } else { p.line }),
            )
            .key(format!("meeting-language-{id}"))
            .label(format!(
                "{name}, {}",
                if selected { "selected" } else { "not selected" }
            ))
            .on_press(move |s: &mut State| s.model_language(id)),
        );
    }
    body = body.child(languages).child(routing);
    if let Some(e) = &models.preference_error {
        body = body.child(roles::META.medium().wrap(e, p.warn).role("alert"));
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
        return body.child(roles::SMALL.wrap("English uses Hex's own model. Nothing to download. Add another language to see its models.", p.soft));
    }
    let total: u64 = visible
        .iter()
        .filter(|m| m.size_tier() == SizeTier::Default && !models.installed.contains(m.id))
        .map(|m| m.bytes())
        .sum();
    body = body
        .child(roles::SMALL.bold().wrap(
            if total == 0 {
                "Everything recommended for these languages is downloaded.".to_string()
            } else {
                format!(
                    "Recommended for these languages: {:.0} MB to download",
                    total as f64 / 1_000_000.0
                )
            },
            p.ink,
        ))
        .child(roles::META.wrap(
            "Models run on this computer. Each download is your choice.",
            p.soft,
        ));
    if models.installed.is_empty() && models.progress.is_empty() {
        body = body.child(roles::META.wrap("No meeting models downloaded yet.", p.soft));
    }
    for m in visible {
        let id = m.id;
        let mut card = col()
            .w_full()
            .gap(5.0)
            .py(12.0)
            .child(hairline(p))
            .child(roles::TITLE.wrap(m.name, p.ink));
        let tier = if m.size_tier() == SizeTier::Default {
            "Recommended"
        } else {
            "Optional"
        };
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
        card = card.child(roles::META.wrap(
            format!(
                "{langs} · {} licence · {}",
                m.licence,
                if m.runtime == Runtime::Hex {
                    "Managed by Hex"
                } else {
                    tier
                }
            ),
            p.faint,
        ));
        if m.runtime == Runtime::Hex {
            card = card.child(roles::META.wrap(m.runtime_note, p.soft));
        } else {
            card = card.child(roles::META.wrap(
                format!("{:.1} MB download", m.bytes() as f64 / 1_000_000.0),
                p.faint,
            ));
            if let Some(memory) = m.minimum_ram_mb {
                card = card.child(
                    roles::META.wrap(format!("Measured model memory: {memory} MB"), p.faint),
                );
            }
            if m.role == model_catalog::Role::LanguageId {
                card = card.child(roles::META.wrap("Tells English from Hindi in each part of the meeting, so each part goes to the right model.", p.soft));
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
                card =
                    card.child(roles::META.wrap("Partial download saved on this device.", p.soft));
                card = card.child(
                    link("Remove partial download", p)
                        .key(format!("model-clear-partial-{id}"))
                        .on_press(move |s: &mut State| s.model_download(id, true)),
                );
            }
            if let Some(error) = models.errors.get(id) {
                card = card.child(roles::META.medium().wrap(error, p.warn).role("alert"));
            }
        }
        body = body.child(card);
    }
    body.child(roles::META.wrap("Downloads stay on this device. Memory figures cover the model process. Your Hex dictation model stays as you chose it.", p.soft))
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

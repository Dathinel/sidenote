use super::{Event, State, hex, roles, theme::Palette, widgets::pressable};
use rune::{El, col, row};
use rune_kit::{Kit, TextInput};
#[cfg(all(windows, feature = "asr-runtime"))]
use sidenote::core::model_catalog::provisioning;
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
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Remove,
    Install,
    #[cfg(all(windows, feature = "asr-runtime"))]
    Automatic(bool),
}

struct Job {
    cancel: Arc<AtomicBool>,
    worker: JoinHandle<()>,
    action: Action,
}

#[derive(Default)]
pub struct Models {
    store: ModelStore,
    #[cfg(all(windows, feature = "asr-runtime"))]
    state_root: std::path::PathBuf,
    #[cfg(all(windows, feature = "asr-runtime"))]
    provisioning: Option<std::fs::File>,
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
    pub fn has_local_files(&self, id: &str) -> bool {
        self.installed.contains(id) || self.partial.contains(id)
    }
    pub fn load(root: &Path, config: &sidenote::core::Config) -> Self {
        let (languages, preference_error) = match model_catalog::load_meeting_languages(root) {
            Ok(languages) => (languages, None),
            Err(e) => (
                vec!["en".into()],
                Some(format!("Could not read meeting languages: {e}")),
            ),
        };
        let model_root = if cfg!(all(windows, feature = "asr-runtime"))
            && !config.pipeline.model_state_dir.as_os_str().is_empty()
        {
            config.pipeline.model_state_dir.clone()
        } else {
            root.to_owned()
        };
        let store = ModelStore::new(model_root);
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
            store,
            #[cfg(all(windows, feature = "asr-runtime"))]
            state_root: root.to_owned(),
            languages,
            installed,
            partial,
            preference_error,
            #[cfg(all(windows, feature = "asr-runtime"))]
            provisioning: None,
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
                let installed_by_request = self.jobs.remove(id).is_some_and(|job| {
                    let requested =
                        job.action == Action::Install && !job.cancel.load(Ordering::Relaxed);
                    let _ = job.worker.join();
                    requested
                });
                self.progress.remove(id);
                let store = &self.store;
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
                        if installed_by_request {
                            self.remember_choice(id, false);
                        }
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                        // A failed cancellation preference can still need to be shown.
                    }
                    Err(error) => {
                        self.errors.insert(id, error.to_string());
                    }
                }
            }
        }
    }

    fn start(&mut self, id: &'static str, action: Action, notify: impl Fn(Msg) + Send + 'static) {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let store = self.store.clone();
        #[cfg(all(windows, feature = "asr-runtime"))]
        let state_root = self.state_root.clone();
        self.errors.remove(id);
        self.progress.insert(
            id,
            Progress {
                downloaded: store.stored_bytes(id).unwrap_or(0),
                total: model_catalog::model(id).map_or(0, |m| m.bytes()),
                phase: Phase::Downloading,
            },
        );
        let worker = thread::spawn(move || {
            let progress = |p| notify(Msg::Progress(id, p));
            let result = match action {
                Action::Remove => store.remove(id),
                Action::Install => store.install(id, &worker_cancel, progress),
                #[cfg(all(windows, feature = "asr-runtime"))]
                Action::Automatic(cache_only) => provisioning::install(
                    &store,
                    &state_root,
                    id,
                    &worker_cancel,
                    cache_only,
                    progress,
                ),
            };
            notify(Msg::Finished(id, result));
        });
        self.jobs.insert(
            id,
            Job {
                cancel,
                worker,
                action,
            },
        );
    }

    fn request_download(
        &mut self,
        id: &'static str,
        remove: bool,
        notify: impl Fn(Msg) + Send + 'static,
    ) -> bool {
        if self.scene || self.jobs.contains_key(id) || (remove && !self.remember_choice(id, true)) {
            return false;
        }
        self.start(
            id,
            if remove {
                Action::Remove
            } else {
                Action::Install
            },
            notify,
        );
        true
    }

    fn remember_choice(&mut self, id: &'static str, off: bool) -> bool {
        #[cfg(all(windows, feature = "asr-runtime"))]
        if let Err(error) = provisioning::opt_out(&self.store, id, off) {
            self.preference_error = Some(format!("Could not save the model choice: {error}"));
            return false;
        }
        let _ = (id, off);
        true
    }

    fn cancel(&mut self, id: &'static str) {
        if let Some(job) = self.jobs.get(id) {
            job.cancel.store(true, Ordering::Relaxed);
            if !self.remember_choice(id, true) {
                self.errors.insert(
                    id,
                    "Download stopped. This choice could not be saved for restart.".into(),
                );
            }
        }
    }

    #[cfg(all(windows, feature = "asr-runtime"))]
    fn provision(
        &mut self,
        config: &sidenote::core::Config,
        recording: bool,
        cache_only: bool,
        notify: impl Fn(Msg) + Send + 'static,
    ) -> bool {
        if self.scene {
            return false;
        }
        if let Some(job) = self
            .jobs
            .values()
            .find(|job| matches!(job.action, Action::Automatic(_)))
        {
            if recording || !provisioning::enabled(config) {
                job.cancel.store(true, Ordering::Relaxed);
            }
            return false;
        }
        let Some(id) =
            provisioning::next_pack(&self.store, config, |id| self.errors.contains_key(id))
        else {
            self.provisioning = None;
            return false;
        };
        if self.provisioning.is_none() {
            match provisioning::reserve(&self.store) {
                Ok(lock) => self.provisioning = Some(lock),
                Err(error) if matches!(error.raw_os_error(), Some(32 | 33)) => return false,
                Err(error) => {
                    self.errors.insert(id, error.to_string());
                    return true;
                }
            }
        }
        if recording
            || !self.jobs.is_empty()
            || (!cache_only && !provisioning::recording_idle(&self.state_root))
        {
            return false;
        }
        self.start(id, Action::Automatic(cache_only), notify);
        true
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
        self.models.request_download(id, remove, model_notify(self));
    }

    #[cfg(all(windows, feature = "asr-runtime"))]
    pub(super) fn provision_models(&mut self, tx: &mut rune::Tx) -> bool {
        let recording = self.hex_record_busy();
        let notify = model_notify(self);
        let changed = self
            .models
            .provision(&self.config, recording, false, notify);
        if self.models.provisioning.is_some() && self.models.jobs.is_empty() {
            tx.wake_after(std::time::Duration::from_secs(1));
        }
        changed
    }
}

fn model_notify(s: &State) -> impl Fn(Msg) + Send + 'static {
    let sender = s.events_tx.clone();
    let waker = s.waker.clone();
    move |msg| {
        let _ = sender.send(Event::Models(msg));
        if let Some(wake) = &waker {
            wake();
        }
    }
}
/// Runs one Settings or automatic job in an isolated proof process.
#[cfg(all(windows, feature = "asr-runtime"))]
pub(crate) fn provision_for_proof(
    root: &Path,
    mode: &str,
    cancel_at: Option<u64>,
) -> std::io::Result<()> {
    use std::{
        io,
        time::{Duration, Instant},
    };
    let config = sidenote::core::Config::load_in(root)?;
    if !root.is_absolute() || !config.pipeline.model_state_dir.as_os_str().is_empty() {
        return Err(io::Error::other(
            "The model check needs an absolute, isolated state folder",
        ));
    }
    if mode != "cache"
        && !root
            .canonicalize()?
            .starts_with(std::env::temp_dir().canonicalize()?)
    {
        return Err(io::Error::other(
            "The catalog check needs a temporary state folder",
        ));
    }
    let mut models = Models::load(root, &config);
    let (tx, rx) = std::sync::mpsc::channel();
    let notify = || {
        let sender = tx.clone();
        move |msg| {
            let _ = sender.send(msg);
        }
    };
    if matches!(mode, "cache" | "catalog") {
        let deadline = Instant::now() + Duration::from_secs(30);
        while !models.provision(&config, false, mode == "cache", notify()) {
            if mode == "cache"
                || Instant::now() >= deadline
                || provisioning::next_pack(&models.store, &config, |id| {
                    models.errors.contains_key(id)
                })
                .is_none()
            {
                return Err(io::Error::other("No automatic model install could start"));
            }
            thread::sleep(Duration::from_millis(50));
        }
    } else {
        let (action, id) = mode
            .split_once(':')
            .ok_or_else(|| io::Error::other("Unknown model check action"))?;
        let id = provisioning::PACKS
            .into_iter()
            .find(|pack| *pack == id)
            .ok_or_else(|| io::Error::other("Unknown automatic model pack"))?;
        if !matches!(action, "install" | "remove")
            || !models.request_download(id, action == "remove", notify())
        {
            return Err(io::Error::other(
                "Could not start the requested Settings action",
            ));
        }
    }
    drop(tx);
    for msg in rx {
        if let Msg::Progress(id, ref progress) = msg {
            println!(
                "{id} {:?} {}/{}",
                progress.phase, progress.downloaded, progress.total
            );
            if matches!(progress.phase, Phase::Downloading)
                && cancel_at.is_some_and(|at| progress.downloaded >= at)
            {
                models.cancel(id);
            }
        } else if let Msg::Finished(_, ref outcome) = msg {
            let result = outcome
                .as_ref()
                .map(|()| ())
                .map_err(|e| io::Error::new(e.kind(), e.to_string()));
            models.apply(msg);
            return result;
        }
        models.apply(msg);
    }
    Err(std::io::Error::other("Automatic model worker disconnected"))
}

#[cfg(all(test, windows, feature = "asr-runtime"))]
#[test]
fn automatic_model_startup_respects_recording_and_saved_choices() {
    let state = tempfile::tempdir().unwrap();
    let root = state.path();
    let mut config = sidenote::core::Config::load_in(root).unwrap();
    let mut models = Models::load(root, &config);
    assert!(!models.provision(&config, true, true, |_| {}));
    assert!(models.jobs.is_empty());

    let silero = provisioning::PACKS[0];
    assert!(models.provision(&config, false, true, |_| {}));
    assert_eq!(models.jobs.len(), 1);
    assert!(models.progress.contains_key(silero));
    assert!(!models.provision(&config, false, true, |_| {}));
    models.cancel(silero);
    drop(models);

    let mut models = Models::load(root, &config);
    assert!(models.provision(&config, false, true, |_| {}));
    assert!(models.jobs.contains_key(provisioning::PACKS[1]));
    drop(models);

    let mut models = Models::load(root, &config);
    assert!(models.remember_choice(silero, false));
    config.pipeline.language_mode = "forced:te".into();
    assert!(!models.provision(&config, false, true, |_| {}));
    config.pipeline.language_mode = "auto".into();
    config.labs.after_stop_language_routing = false;
    assert!(!models.provision(&config, false, true, |_| {}));
    assert_eq!(
        provisioning::next_pack(&models.store, &sidenote::core::Config::default(), |_| false),
        Some(silero)
    );
}

#[cfg(all(test, windows, feature = "asr-runtime"))]
#[test]
fn provisioning_review_cancel_stops_even_when_saving_fails() {
    let root = tempfile::tempdir().unwrap();
    let config = sidenote::core::Config::default();
    let mut models = Models::load(root.path(), &config);
    let id = provisioning::PACKS[0];
    models.start(id, Action::Automatic(true), |_| {});
    let blocked = root.path().join("not-a-directory");
    std::fs::write(&blocked, b"fixture").unwrap();
    models.store = ModelStore::new(blocked);

    models.cancel(id);

    assert!(models.jobs[id].cancel.load(Ordering::Relaxed));
    assert!(
        models
            .preference_error
            .as_deref()
            .is_some_and(|e| e.contains("Could not save"))
    );
    assert!(models.errors.contains_key(id));
}

#[cfg(all(test, windows, feature = "asr-runtime"))]
#[test]
fn provisioning_review_automatic_packs_keep_their_settings_controls() {
    let mut state = State::new();
    state.models.languages = vec!["en".into()];
    state.hex.shell.model_recommended = true;
    state.models.progress.insert(
        "anuvani-lid-int8",
        Progress {
            downloaded: 10_000,
            total: 169_000_000,
            phase: Phase::Downloading,
        },
    );
    state
        .models
        .errors
        .insert("indicconformer-te-ctc", "Connection failed".into());
    let mut runtime = rune::Runtime::new(
        state,
        |s, _| view(s, super::theme::palette(false), rune::el()),
        (1000.0, 1000.0),
        rune::Env::default(),
    );
    runtime.frame(0.0);
    assert!(runtime.find("model-cancel-anuvani-lid-int8").is_some());
    for id in provisioning::PACKS {
        assert!(
            runtime.find(&format!("speech-model-{id}")).is_some(),
            "{id}"
        );
    }
    assert!(
        runtime
            .find("model-download-indicconformer-te-ctc")
            .is_some()
    );
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
        out = out.child(language_column(s, k, p).padding(14.0));
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
                        roles::CONTROL.line(if selected { "\u{2611}" } else { "\u{2610}" }, p.ink),
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
        return super::hex::settings::panel(p).child(super::hex::settings::setting_row(
            s,
            "speech-models",
            "Transcribe with: Hex",
            "Uses the speech model managed by Hex.",
            roles::CONTROL.line("In use", p.ok),
            false,
            p,
        ));
    }
    catalog_view(s, p, routing, false)
}
pub fn advanced(s: &State, p: Palette) -> El {
    catalog_view(s, p, rune::el(), true)
}
fn automatic_pack(id: &str) -> bool {
    #[cfg(all(windows, feature = "asr-runtime"))]
    return provisioning::PACKS.contains(&id);
    #[cfg(not(all(windows, feature = "asr-runtime")))]
    {
        let _ = id;
        false
    }
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
    // The automatic packs need controls even with English alone selected.
    let others: Vec<&str> = models
        .languages
        .iter()
        .map(String::as_str)
        .filter(|l| *l != "en")
        .collect();
    let visible: Vec<_> = model_catalog::catalog()
        .iter()
        .filter(|m| {
            let automatic = automatic_pack(m.id);
            if advanced {
                return !automatic
                    && (m.role == model_catalog::Role::LanguageId
                        || !cfg!(feature = "asr-runtime"))
                    && (models.installed.contains(m.id) || models.partial.contains(m.id));
            }
            if m.role == model_catalog::Role::LanguageId && !automatic {
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
                && !automatic
                && m.runtime != Runtime::Hex
                && !model_catalog::PROVISIONAL_DEFAULT_IDS.contains(&m.id)
            {
                return false;
            }
            if m.runtime == Runtime::Hex {
                return false;
            }
            // Downloaded models stay listed so they can be removed after a language is dropped.
            if automatic || models.installed.contains(m.id) || models.progress.contains_key(m.id) {
                return true;
            }
            m.languages.iter().any(|l| others.contains(l))
        })
        .collect();
    if visible.is_empty() {
        return body.child(roles::SMALL.wrap(
            "No speech models match this search and the selected languages.",
            p.soft,
        ));
    }
    let mut model_list = col().w_full();
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
                } else if automatic_pack(id) && m.role == model_catalog::Role::LanguageId {
                    "Identifies the spoken language."
                } else if automatic_pack(id) && m.role == model_catalog::Role::VoiceActivity {
                    "Finds speech before transcription."
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
            if m.role == model_catalog::Role::LanguageId && !automatic_pack(id) {
                card = card.child(roles::SMALL.wrap(
                    "Not available yet. The publisher's patched runtime is not bundled.",
                    p.soft,
                ));
                if models.installed.contains(id) || models.partial.contains(id) {
                    card = card.child(
                        crate::ui::widgets::action("Remove", p)
                            .key(format!("model-remove-{id}"))
                            .on_press(move |s: &mut State| s.model_download(id, true)),
                    );
                }
                model_list = model_list
                    .child(
                        row()
                            .key(format!("speech-model-{id}"))
                            .w_full()
                            .flex_wrap()
                            .items_center()
                            .gaps(14.0, 10.0)
                            .padding(14.0)
                            .fill(p.card)
                            .child(info)
                            .child(card),
                    )
                    .child(super::widgets::hairline(p));
                continue;
            }
            if let Some(progress) = models.progress.get(id) {
                if models
                    .jobs
                    .get(id)
                    .is_some_and(|job| job.action == Action::Remove)
                {
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
                                    p.ink,
                                )
                                .role("status"),
                        )
                        .child(
                            crate::ui::widgets::action("Cancel download", p)
                                .key(format!("model-cancel-{id}"))
                                .on_press(move |s: &mut State| s.models.cancel(id)),
                        );
                }
            } else if models.installed.contains(id) {
                card = card.child(
                    row()
                        .gap(14.0)
                        .child(roles::CONTROL.wrap("Installed", p.ink))
                        .child(
                            crate::ui::widgets::action("Remove", p)
                                .key(format!("model-remove-{id}"))
                                .label(format!("Remove {}", m.name))
                                .on_press(move |s: &mut State| s.model_download(id, true)),
                        ),
                );
            } else {
                card = card.child(
                    crate::ui::widgets::action(
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
                    crate::ui::widgets::action("Remove partial download", p)
                        .key(format!("model-clear-partial-{id}"))
                        .on_press(move |s: &mut State| s.model_download(id, true)),
                );
            }
            if let Some(error) = models.errors.get(id) {
                card = card.child(roles::SMALL.medium().wrap(error, p.warn).role("alert"));
            }
        }
        model_list = model_list
            .child(
                row()
                    .key(format!("speech-model-{id}"))
                    .w_full()
                    .flex_wrap()
                    .items_center()
                    .gaps(14.0, 10.0)
                    .padding(14.0)
                    .fill(p.card)
                    .child(info)
                    .child(card),
            )
            .child(super::widgets::hairline(p));
    }
    body = body.child(super::hex::settings::panel(p).child(model_list));
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
        store: ModelStore::new(s.store.root.clone()),
        #[cfg(all(windows, feature = "asr-runtime"))]
        state_root: s.store.root.clone(),
        #[cfg(all(windows, feature = "asr-runtime"))]
        provisioning: None,
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

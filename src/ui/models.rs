use super::{Event, State, hex, roles, theme::Palette, widgets::pressable};
use rune::{El, col, row};
use rune_kit::{Kit, TextInput};
use sidenote::core::model_catalog::{self, ModelStore, Phase, Progress, Runtime, SizeTier};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
};

#[cfg(all(windows, feature = "asr-runtime"))]
pub(crate) mod proof;

const CPU_DOWNLOAD_KEY: &str = "download-cpu-language-packs";
const CPU_DOWNLOAD_LABEL: &str = "Download Hindi and Telugu (567 MB)";

pub enum Msg {
    Progress(&'static str, Progress),
    Finished(&'static str, std::io::Result<()>),
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Action {
    Remove,
    Install,
}

struct Job {
    cancel: Arc<AtomicBool>,
    worker: JoinHandle<()>,
    action: Action,
}

#[derive(Default)]
pub struct Models {
    store: ModelStore,
    languages: Vec<String>,
    installed: HashSet<&'static str>,
    partial: HashSet<&'static str>,
    progress: HashMap<&'static str, Progress>,
    errors: HashMap<&'static str, String>,
    jobs: HashMap<&'static str, Job>,
    pack_queue: VecDeque<&'static str>,
    #[cfg(test)]
    install_fixture: Option<std::sync::mpsc::Sender<&'static str>>,
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
            languages,
            installed,
            partial,
            preference_error,
            progress: HashMap::new(),
            errors: HashMap::new(),
            jobs: HashMap::new(),
            pack_queue: VecDeque::new(),
            #[cfg(test)]
            install_fixture: None,
            scene: false,
        }
    }
    fn apply(&mut self, msg: Msg) -> bool {
        match msg {
            Msg::Progress(id, progress) => {
                self.progress.insert(id, progress);
            }
            Msg::Finished(id, result) => {
                let mut continue_queue = false;
                if let Some(job) = self.jobs.remove(id) {
                    let cancelled = job.cancel.load(Ordering::Relaxed);
                    let _ = job.worker.join();
                    if self.pack_queue.front() == Some(&id) {
                        if cancelled || result.is_err() {
                            self.pack_queue.clear();
                        } else {
                            self.pack_queue.pop_front();
                            continue_queue = !self.pack_queue.is_empty();
                        }
                    }
                }
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
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                    Err(error) => {
                        self.errors.insert(id, error.to_string());
                    }
                }
                return continue_queue;
            }
        }
        false
    }

    fn start(&mut self, id: &'static str, action: Action, notify: impl Fn(Msg) + Send + 'static) {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        let store = self.store.clone();
        #[cfg(test)]
        let install_fixture = self.install_fixture.clone();
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
                Action::Install => {
                    #[cfg(test)]
                    if let Some(fixture) = install_fixture {
                        notify(Msg::Finished(
                            id,
                            fixture.send(id).map_err(std::io::Error::other),
                        ));
                        return;
                    }
                    store.install(id, &worker_cancel, progress)
                }
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
        if self.scene || self.jobs.contains_key(id) {
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

    fn can_download_cpu_packs(&self, config: &sidenote::core::Config) -> bool {
        cfg!(all(windows, feature = "asr-runtime"))
            && config.labs.after_stop_language_routing
            && config.pipeline.enabled()
            && config.pipeline.language_mode == "auto"
            && self.pack_queue.is_empty()
            && self.jobs.keys().all(|id| !cpu_language_pack(id))
            && model_catalog::CPU_LANGUAGE_PACKS
                .iter()
                .any(|id| !self.installed.contains(id))
    }

    fn start_next_cpu_pack(&mut self, notify: impl Fn(Msg) + Send + 'static) {
        if let Some(id) = self.pack_queue.front().copied()
            && !self.request_download(id, false, notify)
        {
            self.pack_queue.clear();
        }
    }

    fn cancel(&mut self, id: &'static str) {
        if let Some(job) = self.jobs.get(id) {
            job.cancel.store(true, Ordering::Relaxed);
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
        if self.models.pack_queue.is_empty() || !cpu_language_pack(id) {
            self.models.request_download(id, remove, model_notify(self));
        }
    }

    fn download_cpu_packs(&mut self) {
        if !self.models.can_download_cpu_packs(&self.config) {
            return;
        }
        self.models.pack_queue = model_catalog::CPU_LANGUAGE_PACKS
            .into_iter()
            .filter(|id| !self.models.installed.contains(id))
            .collect();
        self.models.start_next_cpu_pack(model_notify(self));
    }

    pub(super) fn model_event(&mut self, msg: Msg) {
        if self.models.apply(msg) {
            self.models.start_next_cpu_pack(model_notify(self));
        }
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
#[cfg(all(test, windows, feature = "asr-runtime"))]
#[test]
fn dev40_startup_does_not_download() {
    let mut state = State::new();
    let fixture = tempfile::tempdir().unwrap();
    let root = fixture.path();
    let config = sidenote::core::Config::default();
    state.models = Models::load(root, &config);
    state.config = config;

    // Make the old startup path eligible without allowing an HTTP request.
    let model = model_catalog::model("silero-vad").unwrap();
    let file = &model.files[0];
    let pack = root.join("models").join(model.id);
    std::fs::create_dir_all(&pack).unwrap();
    std::fs::File::create(pack.join(format!("{}.part", file.name)))
        .unwrap()
        .set_len(file.bytes)
        .unwrap();
    std::fs::create_dir_all(root.join("local")).unwrap();
    std::fs::write(
        root.join("local/status.ndjson"),
        serde_json::json!({
            "timestamp": chrono::Utc::now().to_rfc3339(),
            "protocol": 4, "meeting_state": "idle", "status_error": null,
        })
        .to_string(),
    )
    .unwrap();
    state.store.root = root.to_owned();
    let mut runtime = rune::Runtime::new(
        state,
        |_, _| rune::el(),
        (800.0, 600.0),
        rune::Env::default(),
    );
    runtime.poll = Some(super::poll);
    runtime.frame(0.0);
    assert!(
        runtime.state.models.jobs.is_empty(),
        "startup queued a model download"
    );
}

#[cfg(all(test, windows, feature = "asr-runtime"))]
#[test]
fn provisioning_review_cancel_stops_even_when_saving_fails() {
    let root = tempfile::tempdir().unwrap();

    let blocked = root.path().join("not-a-directory");
    std::fs::write(&blocked, b"fixture").unwrap();
    let mut state = State::new();
    state.models = Models::load(&blocked, &sidenote::core::Config::default());
    state.store.root = blocked;
    let id = "silero-vad";
    assert!(state.models.request_download(id, false, |_| {}));
    state.model_language("hi");
    state.models.cancel(id);

    assert!(state.models.jobs[id].cancel.load(Ordering::Relaxed));
    assert!(
        state
            .models
            .preference_error
            .as_deref()
            .is_some_and(|e| e.contains("Could not save meeting languages"))
    );
}

#[cfg(all(test, windows, feature = "asr-runtime"))]
#[test]
fn provisioning_review_cpu_language_packs_keep_their_settings_controls() {
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
    for id in model_catalog::CPU_LANGUAGE_PACKS {
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

#[cfg(all(test, windows, feature = "asr-runtime"))]
#[test]
fn dev40_prompt_matches_routing_and_installed_packs() {
    let mut state = State::new();
    state.config = sidenote::core::Config::default();
    state.models.installed.clear();
    let mut runtime = settings_models_runtime(state);
    assert!(runtime.find(CPU_DOWNLOAD_KEY).is_some());

    runtime.state.config.labs.after_stop_language_routing = false;
    runtime.invalidate();
    runtime.frame(0.01);
    assert!(runtime.find(CPU_DOWNLOAD_KEY).is_none());

    runtime.state.config.labs.after_stop_language_routing = true;
    runtime.state.config.pipeline.language_mode = "forced:te".into();
    runtime.invalidate();
    runtime.frame(0.01);
    assert!(runtime.find(CPU_DOWNLOAD_KEY).is_none());

    runtime.state.config.pipeline.language_mode = "auto".into();
    runtime
        .state
        .models
        .installed
        .extend(model_catalog::CPU_LANGUAGE_PACKS);
    runtime.invalidate();
    runtime.frame(0.01);
    assert!(runtime.find(CPU_DOWNLOAD_KEY).is_none());

    runtime
        .state
        .models
        .installed
        .remove("indicconformer-te-ctc");
    runtime.invalidate();
    runtime.frame(0.01);
    assert!(runtime.find(CPU_DOWNLOAD_KEY).is_some());
}

#[cfg(all(test, windows, feature = "asr-runtime"))]
#[test]
fn dev40_prompt_queues_exactly_four_manual_installs() {
    let mut state = State::new();
    let root = tempfile::tempdir().unwrap();
    state.config = sidenote::core::Config::default();
    state.models = Models::load(root.path(), &state.config);
    let (installed_tx, installed_rx) = std::sync::mpsc::channel();
    state.models.install_fixture = Some(installed_tx);
    let mut runtime = settings_models_runtime(state);
    assert!(click_model_control(&mut runtime, CPU_DOWNLOAD_KEY));
    let timeout = std::time::Duration::from_secs(5);
    for id in [
        "silero-vad",
        "anuvani-lid-int8",
        "indicconformer-hi-ctc",
        "indicconformer-te-ctc",
    ] {
        assert_eq!(runtime.state.models.jobs.len(), 1);
        assert!(
            runtime
                .state
                .models
                .jobs
                .get(id)
                .is_some_and(|job| job.action == Action::Install),
            "expected the manual worker for {id}"
        );
        assert_eq!(installed_rx.recv_timeout(timeout).unwrap(), id);
        loop {
            let event = runtime.state.events_rx.recv_timeout(timeout).unwrap();
            let finished =
                matches!(&event, Event::Models(Msg::Finished(done, Ok(()))) if *done == id);
            runtime.state.apply_event(event);
            if finished {
                break;
            }
        }
    }
    assert!(runtime.state.models.jobs.is_empty());
    assert!(runtime.state.models.pack_queue.is_empty());
    assert!(installed_rx.try_recv().is_err());
}

#[cfg(all(windows, feature = "asr-runtime"))]
fn settings_models_runtime(state: State) -> rune::Runtime<State> {
    let mut runtime = rune::Runtime::new(
        state,
        |s, _| view(s, super::theme::palette(false), rune::el()),
        (1200.0, 1000.0),
        rune::Env::default(),
    );
    runtime.frame(0.0);
    runtime
}

#[cfg(all(windows, feature = "asr-runtime"))]
fn click_model_control(runtime: &mut rune::Runtime<State>, key: &str) -> bool {
    let Some((x, y)) = runtime.find(key) else {
        return false;
    };
    runtime.pointer_move(x, y);
    runtime.pointer_down(x, y);
    runtime.pointer_up(x, y);
    runtime.frame(0.01);
    true
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
fn cpu_language_pack(id: &str) -> bool {
    #[cfg(all(windows, feature = "asr-runtime"))]
    return model_catalog::CPU_LANGUAGE_PACKS.contains(&id);
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
        if models.can_download_cpu_packs(&s.config) {
            body = body.child(
                super::widgets::action(CPU_DOWNLOAD_LABEL, p)
                    .key(CPU_DOWNLOAD_KEY)
                    .on_press(State::download_cpu_packs),
            );
        }
    }
    if let Some(e) = &models.preference_error {
        body = body.child(roles::SMALL.medium().wrap(e, p.warn).role("alert"));
    }
    // The CPU language packs need controls even with English alone selected.
    let others: Vec<&str> = models
        .languages
        .iter()
        .map(String::as_str)
        .filter(|l| *l != "en")
        .collect();
    let visible: Vec<_> = model_catalog::catalog()
        .iter()
        .filter(|m| {
            let cpu_language = cpu_language_pack(m.id);
            if advanced {
                return !cpu_language
                    && (m.role == model_catalog::Role::LanguageId
                        || !cfg!(feature = "asr-runtime"))
                    && (models.installed.contains(m.id) || models.partial.contains(m.id));
            }
            if m.role == model_catalog::Role::LanguageId && !cpu_language {
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
                && !cpu_language
                && m.runtime != Runtime::Hex
                && !model_catalog::PROVISIONAL_DEFAULT_IDS.contains(&m.id)
            {
                return false;
            }
            if m.runtime == Runtime::Hex {
                return false;
            }
            // Downloaded models stay listed so they can be removed after a language is dropped.
            if cpu_language || models.installed.contains(m.id) || models.progress.contains_key(m.id)
            {
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
                } else if cpu_language_pack(id) && m.role == model_catalog::Role::LanguageId {
                    "Identifies the spoken language."
                } else if cpu_language_pack(id) && m.role == model_catalog::Role::VoiceActivity {
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
            if m.role == model_catalog::Role::LanguageId && !cpu_language_pack(id) {
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
            } else if cpu_language_pack(id) && !models.pack_queue.is_empty() {
                card = card.child(roles::CONTROL.wrap(
                    if models.installed.contains(id) {
                        "Installed"
                    } else {
                        "Waiting to download"
                    },
                    p.soft,
                ));
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
                && (!cpu_language_pack(id) || models.pack_queue.is_empty())
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
        languages: vec!["en".into(), "hi".into()],
        scene: true,
        installed: HashSet::new(),
        partial: HashSet::new(),
        progress: HashMap::new(),
        errors: HashMap::new(),
        jobs: HashMap::new(),
        pack_queue: VecDeque::new(),
        #[cfg(test)]
        install_fixture: None,
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

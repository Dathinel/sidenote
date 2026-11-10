//! Settings layout and destinations. Hex keys are routed, never renamed or migrated.
use super::*;
use crate::ui::{ai_notes, icons, nav, tasks};
use sidenote::core::{Feature as AppFeature, UiPrefs};
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Settings {
    pub last_tabs: BTreeMap<String, usize>,
    back: Option<ReturnTo>,
    pub advanced: HashSet<&'static str>,
    pub highlighted: Option<(String, Instant)>,
    pub saved: Option<Instant>,
    pub helper_query: TextEdit,
    pub model_query: TextEdit,
    pub language_query: TextEdit,
    pub recommended: bool,
    pub model_recommended: bool,
}

struct ReturnTo {
    page: nav::Page,
    selected: Option<String>,
    filter: nav::Filter,
    live: bool,
    transcript: bool,
    meeting_tab: crate::ui::meeting::Tab,
}
pub(super) fn remember_back(s: &mut State) {
    s.hex.shell.back = Some(ReturnTo {
        page: s.page(),
        selected: s.selected.clone(),
        filter: s.filter.clone(),
        live: s.live_open,
        transcript: s.transcript_open,
        meeting_tab: s.meeting_tab,
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    General,
    Meetings,
    Prompts,
    People,
    Hex,
    Helpers,
    Appearance,
    Sync,
    Updates,
}
impl Page {
    const ALL: [Self; 9] = [
        Self::General,
        Self::Meetings,
        Self::Prompts,
        Self::People,
        Self::Hex,
        Self::Helpers,
        Self::Appearance,
        Self::Sync,
        Self::Updates,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Meetings => "Meetings",
            Self::Prompts => "Prompt library",
            Self::People => "People",
            Self::Hex => "Hex and dictation",
            Self::Helpers => "AI helpers",
            Self::Appearance => "Appearance and startup",
            Self::Sync => "Sync",
            Self::Updates => "Updates",
        }
    }
    fn tabs(self) -> &'static [&'static str] {
        match self {
            Self::Meetings => &["Recording", "Summaries", "Tasks"],
            Self::Helpers => &["Helpers", "Speech model", "Advanced"],
            _ => &[],
        }
    }
    fn section(self, tab: usize) -> Section {
        match self {
            Self::General => Section::Identity,
            Self::Meetings => match tab {
                1 => Section::AiNotes,
                2 => Section::Features,
                _ => Section::Meetings,
            },
            Self::Prompts => Section::PromptLibrary,
            Self::People => Section::Teams,
            Self::Hex => Section::Dictation,
            Self::Helpers => match tab {
                1 => Section::MeetingModels,
                2 => Section::HelpersAdvanced,
                _ => Section::Connect,
            },
            Self::Appearance => Section::General,
            Self::Sync => Section::Sync,
            Self::Updates => Section::Updates,
        }
    }
    fn icon(self) -> &'static rune::Icon {
        match self {
            Self::General => &icons::SETTINGS,
            Self::Meetings => &icons::MEETINGS,
            Self::Prompts => &icons::PROMPTS,
            Self::People => &icons::PEOPLE,
            Self::Hex => &icons::MIC,
            Self::Helpers => &icons::BOT,
            Self::Appearance => &icons::PALETTE,
            Self::Sync => &icons::DEVICE,
            Self::Updates => &icons::UPDATES,
        }
    }
}

fn current(section: Section) -> (Page, usize) {
    match section {
        Section::Identity => (Page::General, 0),
        Section::Meetings | Section::Vocabulary => (Page::Meetings, 0),
        Section::AiNotes => (Page::Meetings, 1),
        Section::Features => (Page::Meetings, 2),
        Section::PromptLibrary => (Page::Prompts, 0),
        Section::Teams => (Page::People, 0),
        Section::Model | Section::MeetingModels => (Page::Helpers, 1),
        Section::Connect => (Page::Helpers, 0),
        Section::HelpersAdvanced => (Page::Helpers, 2),
        Section::General => (Page::Appearance, 0),
        Section::Sync => (Page::Sync, 0),
        Section::Updates | Section::Advanced => (Page::Updates, 0),
        Section::Dictation | Section::Shortcuts => (Page::Hex, 0),
    }
}

fn navigate(s: &mut State, page: Page, tab: usize) {
    let tab = tab.min(page.tabs().len().saturating_sub(1));
    s.hex.section = page.section(tab);
    s.hex.select_open = None;
    s.hex.filter = TextEdit::default();
    s.hex.shell.last_tabs.insert(page.name().into(), tab);
    if rune::tooling::headless_run().is_none() {
        let mut prefs = UiPrefs::load(&s.store.root);
        prefs.settings_tabs = s.hex.shell.last_tabs.clone();
        let _ = prefs.save(&s.store.root);
    }
    super::super::scene_scroll(s, "hex-top");
}

fn back(s: &mut State) {
    let dest = s.hex.shell.back.take();
    s.go_home();
    if let Some(dest) = dest {
        s.filter = dest.filter;
        s.selected = dest.selected;
        s.live_open = dest.live;
        s.transcript_open = dest.transcript;
        s.meeting_tab = dest.meeting_tab;
        match dest.page {
            nav::Page::Tasks => s.view = tasks::View::Tasks,
            nav::Page::Memory => s.memory_open = true,
            nav::Page::Check => s.check_open = true,
            _ => {}
        }
        s.refresh_list();
    }
}

pub fn tick(s: &mut State) -> bool {
    let mut changed = false;
    if s.on_hex() {
        let (page, tab) = current(s.hex.section);
        if s.hex.shell.last_tabs.get(page.name()) != Some(&tab) {
            s.hex.shell.last_tabs.insert(page.name().into(), tab);
            if rune::tooling::headless_run().is_none() {
                let mut prefs = UiPrefs::load(&s.store.root);
                prefs.settings_tabs = s.hex.shell.last_tabs.clone();
                let _ = prefs.save(&s.store.root);
            }
        }
    }
    if s.hex
        .shell
        .highlighted
        .as_ref()
        .is_some_and(|(_, until)| Instant::now() >= *until)
    {
        s.hex.shell.highlighted = None;
        changed = true;
    }
    if s.hex
        .shell
        .saved
        .is_some_and(|until| Instant::now() >= until)
    {
        s.hex.shell.saved = None;
        s.ai_notes.notice = None;
        changed = true;
    }
    changed
}
pub(crate) fn highlight(s: &State, key: &str, p: Palette) -> Color {
    if s.hex
        .shell
        .highlighted
        .as_ref()
        .is_some_and(|(id, until)| id == key && Instant::now() < *until)
    {
        p.hit
    } else {
        p.card
    }
}
pub(crate) fn group_highlight(s: &State, key: &str, p: Palette) -> Color {
    if s.hex
        .shell
        .highlighted
        .as_ref()
        .is_some_and(|(id, until)| id == key && Instant::now() < *until)
    {
        p.hit
    } else {
        p.bg.alpha(0.0)
    }
}
pub(crate) use crate::ui::cards::{icon_tile, panel};
pub(crate) fn pc(p: Palette) -> El {
    roles::META
        .line("This PC", p.soft)
        .px(6.0)
        .py(2.0)
        .radius(4.0)
        .fill(p.chip)
        .help("Kept on this PC only. Your other devices have their own.")
}
pub(crate) fn row_icon(key: &str) -> &'static rune::Icon {
    match key.trim_start_matches("hs-") {
        "meetings.auto_start" => &icons::PHONE_CALL,
        "meetings.record_my_microphone" => &icons::MIC,
        "languages-row" | "meeting-language-mode" => &icons::LANGUAGES,
        "vocabulary-row" => &icons::SPELLING,
        "feature-Memory" => &icons::BRAIN,
        "keep-audio" => &icons::AUDIO_LINES,
        "meetings.retention" => &icons::TRASH,
        "meetings.silence_threshold" => &icons::VOLUME_X,
        "meetings.output_folder" | "sync-folder" => &icons::FOLDER,
        "notes-identity" => &icons::USER,
        "summary-template" => &icons::FILE_TEXT,
        "notes-options" => &icons::SUMMARY,
        "notes-privacy" => &icons::SHIELD_CHECK,
        "dictation.shortcut" => &icons::KEYBOARD,
        "shortcuts.toggle_meeting" => &icons::CIRCLE_STOP,
        "dictation.remove_filler_words" => &icons::ERASER,
        "shortcuts.paste_last" => &icons::CLIPBOARD_PASTE,
        "dictation.max_seconds" => &icons::TIMER,
        "dictation.paste_method" => &icons::CLIPBOARD,
        "dictation.language" => &icons::SPEECH,
        "general.theme" => &icons::PALETTE,
        "general.launch_at_login" => &icons::POWER,
        "general.sound_effects" => &icons::VOLUME_2,
        "labs.meeting_finished_event" => &icons::FLASK,
        "labs.live_captions" => &icons::CAPTIONS,
        "labs.gpu_overlay" => &icons::CPU,
        "cloud.api_key" => &icons::KEY_ROUND,
        "about.build" => &icons::PACKAGE,
        "diagnostics.log_level" => &icons::FILE_CLOCK,
        "network.proxy" => &icons::GLOBE,
        "hex-status" => &icons::AUDIO_WAVEFORM,
        "feature-Daily check" => &icons::CALENDAR_CHECK,
        "feature-Tasks" => &icons::LIST_TODO,
        "feature-Projects" => &icons::FOLDERS,
        "feature-Other devices" => &icons::RADIO_TOWER,
        "this-pc-name" => &icons::MONITOR,
        "connect-Claude Code" => &icons::CONNECTED,
        "connect-Claude Desktop" => &icons::MONITOR,
        "connect-Codex" => &icons::APP_WINDOW,
        "connect-Cursor" => &icons::MOUSE_POINTER,
        "connect-OpenCode" => &icons::NETWORK,
        "notes-helper-0" => &icons::TERMINAL,
        "notes-helper-1" => &icons::CODE_XML,
        "notes-helper-2" => &icons::SQUARE_TERMINAL,
        key if key.starts_with("sync-device-") => &icons::SMARTPHONE,
        _ => &icons::SETTINGS,
    }
}
pub(crate) fn setting_row(
    s: &State,
    key: &str,
    name: &str,
    help: &str,
    control: El,
    local: bool,
    p: Palette,
) -> El {
    let mut title = row()
        .items_center()
        .flex_wrap()
        .gap(8.0)
        .child(roles::NAME.wrap(name, p.ink));
    if local {
        title = title.child(pc(p));
    }
    crate::ui::cards::row_shell()
        .key(key)
        .flex_wrap()
        .fill(highlight(s, key, p))
        .child(icon_tile(row_icon(key), p))
        .child(crate::ui::cards::copy(title, help, p))
        .child(control)
}
pub(crate) fn card(title: &str, body: El, p: Palette) -> El {
    col()
        .w_full()
        .gap(8.0)
        .mt(18.0)
        .child(roles::GROUP.wrap(title, p.soft))
        .child(panel(p).child(body.w_full()))
}
fn advanced(s: &State, body: El, p: Palette) -> El {
    let page = current(s.hex.section).0.name();
    let open = s.hex.shell.advanced.contains(page) || s.hex.section == Section::Advanced;
    let mut out = col().w_full().mt(18.0).child(
        crate::ui::cards::fold_control("Advanced", "settings-advanced", open, p).on_press(
            move |s: &mut State| {
                if !s.hex.shell.advanced.remove(page) {
                    s.hex.shell.advanced.insert(page);
                }
                if s.hex.section == Section::Advanced {
                    s.hex.section = Section::Updates;
                }
            },
        ),
    );
    if open {
        out = out.child(body);
    }
    out
}

#[derive(Clone)]
struct Destination {
    key: String,
    name: String,
    words: String,
    page: Page,
    tab: usize,
    advanced: bool,
}
fn entry(
    key: &str,
    name: &str,
    words: &str,
    page: Page,
    tab: usize,
    advanced: bool,
) -> Destination {
    Destination {
        key: key.into(),
        name: name.into(),
        words: words.into(),
        page,
        tab,
        advanced,
    }
}
pub(super) fn is_meeting_setting(st: &Setting) -> bool {
    location(st).0 == Page::Meetings
}
fn location(st: &Setting) -> (Page, usize, bool) {
    let key = st.key.as_str();
    if matches!(
        key,
        "meetings.audio_retention"
            | "labs.expire_meeting_audio"
            | "meetings.retention"
            | "meetings.silence_threshold"
            | "meetings.output_folder"
    ) {
        return (Page::Meetings, 0, true);
    }
    match place(st) {
        Place::In(Section::Meetings) => (Page::Meetings, 0, false),
        Place::In(Section::General) => (Page::Appearance, 0, false),
        Place::In(Section::Dictation) => (Page::Hex, 0, !key.contains("filler")),
        Place::In(Section::Shortcuts) => (Page::Hex, 0, key.contains("paste_last")),
        _ => (Page::Updates, 0, true),
    }
}
pub(super) fn setting_name(st: &Setting) -> String {
    match st.key.as_str() {
        "meetings.auto_start" => "Start when a call starts".into(),
        "meetings.record_my_microphone" => "Record my microphone".into(),
        "shortcuts.toggle_meeting" => "Start or stop a meeting".into(),
        "shortcuts.paste_last" => "Paste last shortcut".into(),
        "dictation.shortcut" => "Dictate".into(),
        "dictation.language" => "Dictation language".into(),
        "general.launch_at_login" => "Open Hex when you sign in to Windows".into(),
        _ => human(st, place(st)),
    }
}
pub(super) fn setting_help(st: &Setting) -> String {
    match st.key.as_str() {
        "meetings.auto_start" => "Sidenote starts recording on its own when a call starts.".into(),
        _ => one_line(&st.about),
    }
}
fn app_index() -> Vec<Destination> {
    vec![
        entry(
            "notes-identity",
            "Your name",
            "identity profile",
            Page::General,
            0,
            false,
        ),
        entry(
            "notes-options",
            "Write a meeting summary when a meeting ends",
            "AI notes Notes automatic summary",
            Page::Meetings,
            1,
            false,
        ),
        entry(
            "summary-template",
            "Summary template",
            "AI notes Notes instructions prompt",
            Page::Meetings,
            1,
            false,
        ),
        entry(
            "notes-scripts-card",
            "Summaries from your own script",
            "AI notes scripts engine",
            Page::Meetings,
            1,
            true,
        ),
        entry(
            "feature-Memory",
            "Remember across meetings",
            "memory",
            Page::Meetings,
            1,
            false,
        ),
        entry(
            "feature-Daily check",
            "Daily check",
            "tasks",
            Page::Meetings,
            2,
            false,
        ),
        entry(
            "feature-Tasks",
            "Tasks",
            "features",
            Page::Meetings,
            2,
            true,
        ),
        entry(
            "feature-Projects",
            "Projects",
            "features group projects by",
            Page::Meetings,
            2,
            true,
        ),
        entry(
            "agent-instructions",
            "Meeting summary instructions",
            "AI notes Notes prompt library instructions tasks",
            Page::Prompts,
            0,
            false,
        ),
        entry(
            "settings-teams",
            "Teams",
            "Teams people members aliases",
            Page::People,
            0,
            false,
        ),
        entry(
            "hex-vocabulary",
            "Names and terms",
            "Vocabulary spelling",
            Page::Meetings,
            0,
            false,
        ),
        entry(
            "spoken-languages",
            "Languages you speak",
            "languages meeting languages",
            Page::Meetings,
            0,
            false,
        ),
        entry(
            "ai-notes-settings",
            "AI helpers",
            "Claude Codex OpenCode AI notes helpers",
            Page::Helpers,
            0,
            false,
        ),
        entry(
            "mcp-connect",
            "Connect AI apps",
            "MCP Claude Desktop Cursor",
            Page::Helpers,
            0,
            false,
        ),
        entry(
            "speech-models",
            "Speech model",
            "Model languages download",
            Page::Helpers,
            1,
            false,
        ),
        entry(
            "helper-advanced",
            "Model per helper",
            "AI notes model sample sign in safety rules",
            Page::Helpers,
            2,
            false,
        ),
        entry(
            "meeting-language-mode",
            "Language handling",
            "routing pipeline",
            Page::Helpers,
            2,
            false,
        ),
        entry(
            "speech-model-anuvani-lid-int8",
            "Anuvani language identification",
            "language identification detection",
            Page::Helpers,
            2,
            true,
        ),
        entry(
            "legacy-hex-models",
            "Hex dictation model",
            "model",
            Page::Helpers,
            2,
            true,
        ),
        entry(
            "feature-Other devices",
            "Other devices",
            "sync",
            Page::Sync,
            0,
            false,
        ),
        entry(
            "sync-folder",
            "Sync folder",
            "data directory",
            Page::Sync,
            0,
            true,
        ),
        entry(
            "updates-section",
            "Check for updates",
            "version what changed",
            Page::Updates,
            0,
            false,
        ),
    ]
}
fn index(s: &State) -> Vec<Destination> {
    let mut entries = app_index();
    entries.retain(|dest| {
        dest.key != "notes-scripts-card"
            || s.ai_notes
                .settings
                .as_ref()
                .is_some_and(|settings| settings.script.is_some())
    });
    entries.extend(s.hex.settings.iter().map(|st| {
        let (page, tab, advanced) = location(st);
        let key = if matches!(
            st.key.as_str(),
            "meetings.audio_retention" | "labs.expire_meeting_audio"
        ) {
            "keep-audio".into()
        } else {
            format!("hs-{}", st.key)
        };
        entry(
            &key,
            &if key == "keep-audio" {
                "Keep audio".into()
            } else {
                setting_name(st)
            },
            &format!("{} {}", st.key, st.about),
            page,
            tab,
            advanced,
        )
    }));
    entries
}
fn matches(item: &Destination, query: &str) -> bool {
    let hay = format!("{} {} {}", item.name, item.words, item.page.name()).to_lowercase();
    query
        .to_lowercase()
        .split_whitespace()
        .all(|word| hay.contains(word))
}
fn results(s: &State) -> Vec<Destination> {
    let words = s.hex.filter.text.to_lowercase();
    index(s)
        .into_iter()
        .filter(|item| matches(item, &words))
        .collect()
}
fn jump(s: &mut State, dest: Destination) {
    navigate(s, dest.page, dest.tab);
    if dest.advanced {
        s.hex.shell.advanced.insert(dest.page.name());
    }
    if dest.key == "hex-vocabulary" {
        s.hex.open_groups.insert("vocabulary-edit");
    }
    s.hex.shell.highlighted = Some((dest.key.clone(), Instant::now() + Duration::from_secs(2)));
    super::super::scene_scroll(s, &dest.key);
}

pub fn sidebar(s: &State, k: &Kit, p: Palette) -> El {
    let compact = s.app_w.get() < 700.0;
    let width = if compact { 184.0 } else { 248.0 };
    let name = s
        .ai_notes
        .settings
        .as_ref()
        .map(|settings| settings.your_name.as_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("You");
    let mut list = col()
        .w_full()
        .gap(3.0)
        .padding(12.0)
        .child(
            pressable(
                row()
                    .items_center()
                    .gap(8.0)
                    .py(8.0)
                    .child(icons::draw(&icons::CHEVRON_LEFT, 18.0, p.soft))
                    .child(roles::CONTROL.line("Back", p.soft)),
            )
            .key("settings-back")
            .label("Back (Esc)")
            .on_press(back),
        )
        .child(
            col()
                .w_full()
                .gap(6.0)
                .py(16.0)
                .child(super::super::nav::profile_avatar(name, p))
                .child(
                    roles::NAME.wrap(
                        s.ai_notes
                            .settings
                            .as_ref()
                            .map(|settings| settings.your_name.as_str())
                            .filter(|name| !name.is_empty())
                            .unwrap_or("You"),
                        p.ink,
                    ),
                )
                .child(roles::META.wrap(
                    if rune::tooling::headless_run().is_some() {
                        "This PC: DEMO-PC".into()
                    } else {
                        format!(
                            "This PC: {}",
                            std::env::var("COMPUTERNAME").unwrap_or_default()
                        )
                    },
                    p.soft,
                )),
        )
        .child(
            TextInput::new(&s.hex.filter)
                .leading_icon(&icons::SEARCH)
                .leading_icon_size(16.0)
                .trailing(super::super::widgets::keycap("/", p))
                .placeholder("Search settings")
                .label("Search settings")
                .height(34.0)
                .on_edit(|s: &mut State| &mut s.hex.filter)
                .on_submit(|s: &mut State, _| {
                    if let Some(dest) = results(s).into_iter().next() {
                        jump(s, dest);
                    }
                })
                .build(k)
                .key("hex-filter")
                .w_full()
                .mb(10.0),
        );
    if !s.hex.filter.text.trim().is_empty() {
        let matches = results(s);
        if matches.is_empty() {
            list = list.child(roles::SMALL.wrap(
                format!(
                    "No setting matches \"{}\". Try a shorter word.",
                    s.hex.filter.text
                ),
                p.soft,
            ));
        }
        for dest in matches {
            let path = path(dest.page, dest.tab);
            list = list.child(
                pressable(
                    col()
                        .w_full()
                        .gap(4.0)
                        .padding(8.0)
                        .child(roles::NAME.wrap(&dest.name, p.ink))
                        .child(roles::META.wrap(path, p.soft)),
                )
                .key(format!("find-setting-{}", dest.key))
                .radius(6.0)
                .hover(p.chip)
                .on_press(move |s: &mut State| jump(s, dest.clone())),
            );
        }
    } else {
        let (active, _) = current(s.hex.section);
        for (i, page) in Page::ALL.into_iter().enumerate() {
            if i == 0 || i == 4 {
                list = list.child(
                    roles::GROUP
                        .line(if i == 0 { "You" } else { "This PC" }, p.faint)
                        .mt(16.0)
                        .mb(5.0),
                );
            }
            list = list.child(
                pressable(
                    row()
                        .w_full()
                        .min_h(38.0)
                        .items_center()
                        .gap(10.0)
                        .px(8.0)
                        .py(7.0)
                        .child(icons::draw(page.icon(), 18.0, p.soft))
                        .child(if active == page {
                            roles::NAME
                                .line(page.name(), p.ink)
                                .grow(1.0)
                                .help(page.name())
                        } else {
                            roles::CONTROL
                                .line(page.name(), p.soft)
                                .grow(1.0)
                                .help(page.name())
                        }),
                )
                .key(format!("settings-page-{}", page.name()))
                .label(page.name())
                .help(page.name())
                .radius(8.0)
                .fill(if active == page { p.sel } else { p.side })
                .hover(p.chip)
                .on_press(move |s: &mut State| {
                    let tab = s.hex.shell.last_tabs.get(page.name()).copied().unwrap_or(0);
                    navigate(s, page, tab);
                }),
            );
        }
    }
    col()
        .key("settings-sidebar")
        .w(width)
        .h_full()
        .min_h(0.0)
        .fill(p.side)
        .child(scroll(list, "settings-list-scroll"))
}
fn path(page: Page, tab: usize) -> String {
    page.tabs().get(tab).map_or_else(
        || page.name().into(),
        |name| format!("{} / {name}", page.name()),
    )
}
fn tab_bar(s: &State, p: Palette) -> El {
    let (page, active) = current(s.hex.section);
    let mut tabs = row()
        .key("settings-tabs")
        .role("tablist")
        .label(format!("{} sections", page.name()))
        .w_full()
        .gap(20.0)
        .focus_group();
    for (tab, name) in page.tabs().iter().enumerate() {
        tabs = tabs.child(
            pressable(
                col()
                    .gap(12.0)
                    .pt(14.0)
                    .child(if tab == active {
                        roles::NAME.wrap(*name, p.ink)
                    } else {
                        roles::CONTROL.wrap(*name, p.soft)
                    })
                    .child(rune::el().w_full().h(2.0).fill(if tab == active {
                        p.accent
                    } else {
                        p.bg
                    })),
            )
            .key(format!("settings-tab-{tab}"))
            .role("tab")
            .focus_selected(tab == active)
            .label(*name)
            .on_press(move |s: &mut State| navigate(s, page, tab)),
        );
    }
    tabs
}
fn hex_rows(
    s: &State,
    k: &Kit,
    p: Palette,
    layer: &mut Vec<El>,
    page: Page,
    tab: usize,
    folded: bool,
) -> El {
    let items: Vec<_> = s
        .hex
        .settings
        .iter()
        .filter(|st| location(st) == (page, tab, folded))
        .collect();
    let mut body = col().w_full();
    if items.iter().any(|st| {
        matches!(
            st.key.as_str(),
            "meetings.audio_retention" | "labs.expire_meeting_audio"
        )
    }) {
        body = body
            .child(
                keep_audio_row(s, k, p, layer)
                    .key("keep-audio")
                    .fill(highlight(s, "keep-audio", p)),
            )
            .child(hairline(p));
    }
    for st in items {
        if matches!(
            st.key.as_str(),
            "meetings.audio_retention" | "labs.expire_meeting_audio"
        ) {
            continue;
        }
        let r = super::setting_row(s, k, st, place(st), p, layer);
        body = body.child(r).child(hairline(p));
    }
    body
}
fn feature(s: &State, k: &Kit, feature: AppFeature, name: &str, help: &str, p: Palette) -> El {
    setting_row(
        s,
        &format!("feature-{}", feature.name()),
        name,
        help,
        Switch::new(s.config.features.enabled(feature))
            .label(name)
            .hide_label()
            .on_toggle(move |s: &mut State| s.toggle_feature(feature))
            .build(k),
        true,
        p,
    )
}
fn devices(s: &State, p: Palette) -> El {
    let name = if rune::tooling::headless_run().is_some() {
        "DEMO-PC".into()
    } else {
        std::env::var("COMPUTERNAME").unwrap_or_else(|_| "This PC".into())
    };
    let mut out = col().w_full().child(setting_row(
        s,
        "this-pc-name",
        "This PC's name",
        "Windows names this PC.",
        roles::CONTROL.wrap(name, p.ink),
        false,
        p,
    ));
    let mut peers = BTreeMap::<&str, usize>::new();
    for meeting in &s.meetings {
        if let Some(device) = meeting.device.as_deref() {
            *peers.entry(device).or_default() += 1;
        }
    }
    for (device, count) in peers {
        out = out.child(hairline(p)).child(setting_row(
            s,
            &format!("sync-device-{device}"),
            device,
            &format!("{count} meetings received. This app does not report a sync time per device."),
            roles::CONTROL.line("Received", p.soft),
            false,
            p,
        ));
    }
    out
}
fn vocabulary(s: &State, k: &Kit, p: Palette) -> El {
    let mut out = col()
        .key("hex-vocabulary")
        .w_full()
        .fill(highlight(s, "hex-vocabulary", p))
        .child(setting_row(
            s,
            "vocabulary-row",
            "Names and terms",
            "Sidenote spells these right in transcripts and summaries.",
            link("Edit terms", p)
                .key("vocabulary-edit")
                .on_press(|s: &mut State| {
                    if !s.hex.open_groups.remove("vocabulary-edit") {
                        s.hex.open_groups.insert("vocabulary-edit");
                    }
                }),
            true,
            p,
        ));
    if s.hex.open_groups.contains("vocabulary-edit") {
        out = out.child(vocabulary_section(s, k, p).padding(14.0));
    }
    out
}
fn body(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>, page: Page, tab: usize) -> El {
    match (page, tab) {
        (Page::General, _) => col().w_full().child(ai_notes::section(s, k, p, ai_notes::Part::Identity))
            .children(ai_notes::setup_card(s, p)),
        (Page::Meetings, 0) => col().w_full()
            .child(card("Recording", hex_rows(s, k, p, layer, page, tab, false), p))
            .child(card("Language and terms", col().w_full().child(super::super::models::language_picker(s,k,p)).child(hairline(p)).child(vocabulary(s,k,p)), p))
            .child(advanced(s, card("Recording on this PC", hex_rows(s, k, p, layer, page, tab, true), p), p)),
        (Page::Meetings, 1) => col().w_full().child(ai_notes::section(s, k, p, ai_notes::Part::Summaries))
            .child(card("Memory", feature(s,k,AppFeature::Memory,"Remember across meetings","Use knowledge from earlier meetings.",p),p))
            .child(advanced(s, ai_notes::section(s,k,p,ai_notes::Part::Scripts),p)),
        (Page::Meetings, _) => col().w_full()
            .child(card("Tasks", feature(s,k,AppFeature::DailyCheck,"Daily check","Sidenote checks your meeting knowledge for gaps and conflicts each day.",p),p))
            .child(advanced(s, card("Sidebar features", col().w_full()
                .child(roles::SMALL.wrap("These existing switches have no other home yet.",p.soft).padding(14.0))
                .child(feature(s,k,AppFeature::Tasks,"Tasks","Show Tasks in the sidebar.",p))
                .child(feature(s,k,AppFeature::Projects,"Projects","Show Projects in the sidebar.",p)),p),p)),
        (Page::Prompts, _) => ai_notes::section(s,k,p,ai_notes::Part::Prompts),
        (Page::People, _) => col().key("settings-teams").fill(group_highlight(s,"settings-teams",p)).w_full().child(card("Our team", super::super::teams::section(s,k,p).padding(14.0),p)),
        (Page::Helpers, 0) => col().w_full().child(ai_notes::section(s,k,p,ai_notes::Part::Helpers))
            .child(card("Connect AI apps",super::super::mcp_connect::section(s,p).key("mcp-connect").fill(highlight(s,"mcp-connect",p)),p)),
        (Page::Helpers, 1) => super::super::models::view(s,p,rune::el()).key("speech-models").fill(highlight(s,"speech-models",p)),
        (Page::Helpers, _) => col().key("helper-advanced").fill(group_highlight(s,"helper-advanced",p)).w_full().child(ai_notes::section(s,k,p,ai_notes::Part::Advanced))
            .child(card("Routing",meeting_languages::section(s,k,p,layer),p))
            .child(advanced(s, col().w_full().key("legacy-hex-models").child(roles::SMALL.wrap("Hex's existing dictation model selector is kept here because it controls a separate engine.",p.soft)).child(model_section(s,p)).child(super::super::models::advanced(s,p)),p)),
        (Page::Hex, _) => col().w_full().child(card("Recording engine",summary_line(s,p),p))
            .child(card("Shortcuts and dictation",hex_rows(s,k,p,layer,page,tab,false),p))
            .child(advanced(s,card("Dictation overrides",hex_rows(s,k,p,layer,page,tab,true),p),p)),
        (Page::Appearance, _) => card("Preferences",hex_rows(s,k,p,layer,page,tab,false),p),
        (Page::Sync, _) => col().w_full().child(card("Devices",devices(s,p).child(hairline(p)).child(feature(s,k,AppFeature::OtherDevices,"Other devices","Show meetings recorded on your other devices.",p)),p))
            .child(roles::SMALL.wrap(format!("{} meetings from other devices.",s.nav.peers),p.soft).mt(12.0))
            .child(advanced(s,card("Sync folder",roles::SMALL.wrap(s.store.root.display().to_string(),p.ink).key("sync-folder").fill(highlight(s,"sync-folder",p)).padding(14.0),p),p)),
        (Page::Updates, _) => col().w_full().child(super::super::updates::section(s,p).fill(highlight(s,"updates-section",p)))
            .child(advanced(s,card("Labs and rarely used",col().w_full().child(hex_rows(s,k,p,layer,page,tab,true)).child(super::super::updates::source(s,p).padding(14.0)),p),p)),
    }
}
pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let (page, tab) = current(s.hex.section);
    let mut doc = col()
        .key("hex-top")
        .w_full()
        .max_w(940.0)
        .px(if narrow { 16.0 } else { 40.0 })
        .pb(40.0);
    if let Some(note_) = s
        .hex
        .page_note()
        .filter(|_| matches!(page, Page::Hex | Page::Appearance | Page::Meetings))
    {
        doc = doc.child(note(note_, p));
    }
    if let Some(problem) = s.hex.settings_note {
        doc = doc.child(note(problem, p));
    }
    if let Some(problem) = &s.feature_error {
        doc = doc.child(roles::SMALL.wrap(problem, p.warn).role("alert"));
    }
    doc = doc.child(body(s, k, p, layer, page, tab));
    let crumb = row()
        .w_full()
        .min_h(58.0)
        .px(24.0)
        .items_center()
        .gap(8.0)
        .child(
            roles::CONTROL
                .wrap(format!("Settings / {}", path(page, tab)), p.ink)
                .grow(1.0),
        )
        .children(
            s.hex
                .shell
                .saved
                .map(|_| roles::SMALL.line("Saved", p.ok).role("status")),
        );
    col()
        .key("hex-pane")
        .w_full()
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .h_full()
        .child(crumb)
        .child(hairline(p))
        .child(
            col().w_full().items_center().child(
                col()
                    .w_full()
                    .max_w(940.0)
                    .px(if narrow { 16.0 } else { 40.0 })
                    .child(
                        roles::SECTION
                            .wrap(page.name(), p.ink)
                            .key("settings-title")
                            .mt(22.0),
                    )
                    .children((!page.tabs().is_empty()).then(|| tab_bar(s, p)))
                    .pb(4.0),
            ),
        )
        .child(scroll(
            col().w_full().items_center().child(doc),
            "hex-scroll",
        ))
}
pub fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        if s.hex.select_open.take().is_some() {
        } else if !s.hex.filter.text.is_empty() {
            s.hex.filter = TextEdit::default();
            tx.blur();
        } else {
            back(s);
            tx.blur();
        }
        return true;
    }
    if e.key == Key::Tab && e.mods.ctrl {
        let (page, tab) = current(s.hex.section);
        if !page.tabs().is_empty() {
            let delta = if e.mods.shift { -1 } else { 1 };
            let next = (tab as isize + delta).rem_euclid(page.tabs().len() as isize) as usize;
            navigate(s, page, next);
            tx.blur();
            return true;
        }
    }
    false
}

/// Synthetic scenes run only with the render harness's isolated state.
pub fn scene(s: &mut State, name: &str) {
    assert!(super::super::isolated());
    super::super::notes_scene::settings(s, "scripts");
    super::super::models::scene(s, "empty");
    s.ai_notes.name = TextEdit::new("Ravi Menon");
    if let Some(settings) = &mut s.ai_notes.settings {
        settings.your_name = "Ravi Menon".into();
        settings.engine = "app".into();
    }
    let (page, tab) = match name.trim_end_matches("-expanded") {
        "settings-general" => (Page::General, 0),
        "settings-recording" => (Page::Meetings, 0),
        "settings-summaries" => (Page::Meetings, 1),
        "settings-tasks" => (Page::Meetings, 2),
        "settings-prompts" => (Page::Prompts, 0),
        "settings-people" => (Page::People, 0),
        "settings-hex" => (Page::Hex, 0),
        "settings-helpers" | "settings-helpers-expanded" => (Page::Helpers, 0),
        "settings-speech" => (Page::Helpers, 1),
        "settings-helper-advanced" => (Page::Helpers, 2),
        "settings-appearance" => (Page::Appearance, 0),
        "settings-sync" => (Page::Sync, 0),
        "settings-updates" => (Page::Updates, 0),
        "settings-search" => (Page::Meetings, 0),
        _ => panic!("Unknown Settings scene"),
    };
    if page == Page::Updates {
        super::super::updates::settings_fixture(s);
    }
    navigate(s, page, tab);
    if name.ends_with("-expanded") {
        s.hex.shell.advanced.insert(page.name());
        s.ai_notes.model_open = true;
        s.connection.show_details = true;
        s.connection.show_all = true;
        if page == Page::Meetings && tab == 0 {
            s.hex.open_groups.insert("languages");
            s.hex.open_groups.insert("vocabulary-edit");
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_old_hex_setting_has_a_reachable_destination() {
        let fixture: Fixture =
            serde_json::from_str(include_str!("../../../tools/ui-fixtures/hex.json")).unwrap();
        for setting in fixture.settings {
            let (page, tab, _) = location(&setting);
            assert!(Page::ALL.contains(&page), "{}", setting.key);
            assert!(tab < page.tabs().len().max(1), "{}", setting.key);
            assert_eq!(current(page.section(tab)), (page, tab));
        }
    }
    #[test]
    fn legacy_search_words_find_the_new_destinations() {
        let entries = app_index();
        for (query, key, page, tab) in [
            ("AI notes", "notes-options", Page::Meetings, 1),
            ("Vocabulary", "hex-vocabulary", Page::Meetings, 0),
            ("Teams", "settings-teams", Page::People, 0),
        ] {
            let result = entries.iter().find(|dest| dest.key == key).unwrap();
            assert!(matches(result, query));
            assert_eq!((result.page, result.tab), (page, tab));
        }
    }
    #[test]
    fn recording_rows_have_distinct_lucide_icons() {
        let keys = [
            "meetings.auto_start",
            "meetings.record_my_microphone",
            "languages-row",
            "vocabulary-row",
            "keep-audio",
            "meetings.retention",
            "meetings.silence_threshold",
            "meetings.output_folder",
        ];
        let mut names = HashSet::new();
        for key in keys {
            let icon = row_icon(key);
            assert!(names.insert(icon.name), "Repeated icon for {key}");
            assert_ne!(icon.name, icons::MEETINGS.name);
        }
        assert_eq!(row_icon("meetings.auto_start").name, "lucide-phone-call");
        assert_eq!(row_icon("notes-identity").name, "lucide-user");
        assert_eq!(row_icon("summary-template").name, "lucide-file-text");
    }
    #[test]
    fn old_deep_links_and_tab_limits_are_preserved() {
        assert_eq!(current(Section::AiNotes), (Page::Meetings, 1));
        assert_eq!(current(Section::Teams), (Page::People, 0));
        assert_eq!(current(Section::Model), (Page::Helpers, 1));
        for page in Page::ALL {
            assert!(page.tabs().len() <= 4);
        }
    }
}

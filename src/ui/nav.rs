//! The left sidebar and the top-right actions.
//!
//! The sidebar holds places and project filters, with one profile button at its foot.
//! It folds to a recording rail during a call and hides fully otherwise.
//! The top right holds one primary "New meeting" button, which becomes "Recording 12:34 | Stop"
//! while Hex records, so the owner can get back to the live page or stop from anywhere.
//! Lucide entries live in icons.rs; the app mark keeps its original colours.
use super::{
    State, hex, roles,
    tasks::View,
    theme::Palette,
    widgets::{dot, pressable, sidebar_icon},
};
use chrono::{DateTime, Local};
use rune::{Color, El, col, el, row};
use sidenote::core::{self, UiPrefs};
use std::collections::HashMap;

/// The open sidebar, and the rail it folds to, in points.
pub const WIDE: f32 = 248.0;
pub const RAIL: f32 = 56.0;
/// A window this wide or narrower starts with the sidebar folded.
pub const COLLAPSE_AT: f32 = 820.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Meetings,
    Tasks,
    Memory,
    Check,
    Hex,
}

/// What the home list shows.
#[derive(Clone, Default, PartialEq, Eq)]
pub enum Filter {
    #[default]
    All,
    /// Meetings whose to-dos mostly sit in this project.
    Project(String),
    Area(String),
    Areas,
    /// Meetings that came from another device.
    Peers,
}

/// Looked up once per rebuild, not per frame: each meeting's project, how many meetings and
/// the newest meeting in each project, and the meetings from other devices.
#[derive(Default)]
pub struct NavCache {
    pub meeting_project: HashMap<String, String>,
    pub project_counts: HashMap<String, usize>,
    /// When each project last met, to list the most recent ones first.
    pub project_latest: HashMap<String, DateTime<Local>>,
    pub peers: usize,
    /// "Show all" under Projects. Kept across rebuilds for the session, never saved.
    pub show_all: bool,
    pub profile_menu: bool,
}

impl State {
    pub fn page(&self) -> Page {
        if self.view == View::Tasks {
            Page::Tasks
        } else if self.on_memory() {
            Page::Memory
        } else if self.on_check() {
            Page::Check
        } else if self.on_hex() {
            Page::Hex
        } else {
            Page::Meetings
        }
    }

    pub fn rebuild_nav(&mut self) {
        let mut cache = NavCache {
            show_all: self.nav.show_all,
            profile_menu: self.nav.profile_menu,
            ..NavCache::default()
        };
        for m in &self.meetings {
            if m.device.is_some() {
                cache.peers += 1;
            }
            let project = self
                .areas
                .meeting_projects
                .get(&m.id)
                .and_then(|v| v.name.clone());
            if let Some(name) = project {
                *cache.project_counts.entry(name.clone()).or_default() += 1;
                if let Some(started) = m.started {
                    let newest = cache.project_latest.entry(name.clone()).or_insert(started);
                    *newest = (*newest).max(started);
                }
                cache.meeting_project.insert(m.id.clone(), name);
            }
        }
        self.nav = cache;
        // A filter on a project that has gone away would show an empty list for nothing.
        if let Filter::Project(name) = &self.filter
            && !self.tasks.names.contains(name)
        {
            self.filter = Filter::All;
        }
        if self.filter == Filter::Peers && self.nav.peers == 0 {
            self.filter = Filter::All;
        }
    }

    pub fn matches_filter(&self, meeting: &core::Meeting) -> bool {
        match &self.filter {
            Filter::All => true,
            Filter::Project(name) => self.nav.meeting_project.get(&meeting.id) == Some(name),
            Filter::Area(id) => self
                .areas
                .activity
                .meetings
                .get(&meeting.id)
                .is_some_and(|t| t.area_id == *id),
            Filter::Areas => true,
            Filter::Peers => meeting.device.is_some(),
        }
    }

    pub fn set_filter(&mut self, filter: Filter) {
        if matches!(filter, Filter::Project(_) | Filter::Area(_) | Filter::Areas)
            && !self.config.features.projects
            || filter == Filter::Peers && !self.config.features.other_devices
        {
            return;
        }
        self.go_home();
        if matches!(filter, Filter::Area(_)) {
            self.areas.show_all = false;
        }
        self.filter = filter;
        self.cursor = 0;
        self.refresh_list();
        self.nav_done();
    }

    /// Meetings, with no filter.
    pub fn go_meetings(&mut self) {
        self.set_filter(Filter::All);
    }

    /// Open: the full sidebar beside the page, or in a narrow window the drawer over it.
    pub fn sidebar_is_open(&self) -> bool {
        if self.narrow.get() {
            self.drawer_open.get()
        } else {
            self.sidebar_open.get()
        }
    }

    /// Opens or folds the sidebar. A choice made in a wide window is remembered for next time;
    /// the drawer of a narrow window is not a preference.
    pub fn set_sidebar(&mut self, open: bool) {
        if self.narrow.get() {
            self.drawer_open.set(open);
            return;
        }
        self.sidebar_open.set(open);
        if rune::tooling::headless_run().is_none() {
            let mut prefs = UiPrefs::load(&self.store.root);
            prefs.sidebar_collapsed = Some(!open);
            // A preference is not worth an error: failing to save only loses the memory of it.
            let _ = prefs.save(&self.store.root);
        }
    }

    pub fn toggle_sidebar(&mut self) {
        let open = !self.sidebar_is_open();
        self.set_sidebar(open);
    }

    /// After going somewhere: a narrow window's drawer closes over the page it opened.
    pub fn nav_done(&mut self) {
        self.drawer_open.set(false);
    }
}

/// The sidebar's width including its divider, for pages that size themselves from the window.
pub fn width(s: &State) -> f32 {
    if s.narrow.get() || !s.sidebar_open.get() {
        if s.hex_recording() { RAIL + 1.0 } else { 0.0 }
    } else {
        WIDE + 1.0
    }
}

// ---- icons ------------------------------------------------------------------------------------

fn icon_meetings(c: Color) -> El {
    super::icons::draw(&super::icons::MEETINGS, 18.0, c)
}
fn icon_tasks(c: Color) -> El {
    super::icons::draw(&super::icons::TASKS, 18.0, c)
}
fn icon_memory(c: Color) -> El {
    super::icons::draw(&super::icons::MEMORY, 18.0, c)
}
fn icon_check(c: Color, _: Color) -> El {
    super::icons::draw(&super::icons::DIARY, 18.0, c)
}
fn icon_device(c: Color) -> El {
    super::icons::draw(&super::icons::DEVICE, 18.0, c)
}
pub fn icon_plus(c: Color) -> El {
    super::icons::draw(&super::icons::PLUS, 16.0, c)
}
pub fn icon_stop(_: f32, c: Color) -> El {
    super::icons::draw(&super::icons::STOP, 16.0, c)
}

// ---- the sidebar ------------------------------------------------------------------------------

/// Row heights: the three places, the tools in the footer, and a project under its label.
const MAIN_H: f32 = 40.0;
/// The open sidebar's side padding, and how far a project sits in from its label.
const PAD: f32 = 10.0;

/// The one quiet mark for something that waits on the user: memory proposals to review, high
/// findings of the daily check in the Diary. Counts live on those pages.
fn signal(c: Color) -> El {
    dot(7.0, c)
}

/// The fill and the hover fill. A selected row keeps its fill under the pointer.
fn fills(active: bool, p: Palette) -> (Color, Color) {
    if active {
        (p.sel, p.sel)
    } else {
        (p.side, p.chip)
    }
}

/// A place's name: 600 in `ink` when active, 500 in `soft` otherwise.
fn place_name(name: &str, active: bool, p: Palette) -> El {
    if active {
        roles::NAME.line(name, p.ink)
    } else {
        roles::CONTROL.line(name, p.soft)
    }
}

/// One of the places in the open sidebar: an icon and a name on a 40-point row.
fn main_row(place: Place, icon: El, p: Palette) -> El {
    let mut r = row()
        .w_full()
        .h(MAIN_H)
        .gap(10.0)
        .items_center()
        .px(PAD)
        .child(icon)
        .child(place_name(place.name, place.active, p));
    if let Some(c) = place.flag {
        r = r.child(signal(c));
    }
    let (bg, hover) = fills(place.active, p);
    pressable(r)
        .key(place.key)
        .label(place.help)
        .radius(8.0)
        .fill(bg)
        .hover(hover)
        .on_press(place.press)
}

/// A place or a tool in the rail: the icon alone, with the place's dot on its corner.
fn rail_item(place: Place, icon: El, p: Palette) -> El {
    let mut b = col().w(40.0).h(36.0).center().child(icon);
    if let Some(c) = place.flag {
        b = b.child(dot(8.0, c).absolute(24.0, 5.0).border(1.5, p.side));
    }
    let (bg, hover) = fills(place.active, p);
    pressable(b)
        .key(place.key)
        .label(place.help.clone())
        .help(place.help)
        .radius(8.0)
        .fill(bg)
        .hover(hover)
        .on_press(place.press)
}

fn toggle(s: &State, p: Palette) -> El {
    let open = s.sidebar_is_open();
    let name = if open {
        "Hide sidebar (Ctrl+\\)"
    } else {
        "Show sidebar (Ctrl+\\)"
    };
    pressable(col().child(sidebar_icon(open, p)))
        .key("sidebar-toggle")
        .label(name)
        .help(name)
        .padding(8.0)
        .radius(8.0)
        .hover(p.chip)
        .on_press(|s: &mut State| s.toggle_sidebar())
}

/// A place or a tool: what it is called, where it goes and the one signal it may carry.
struct Place {
    key: &'static str,
    name: &'static str,
    help: String,
    active: bool,
    icon: fn(Color, Color) -> El,
    flag: Option<Color>,
    press: fn(&mut State),
}

impl Place {
    fn icon(&self, p: Palette) -> El {
        (self.icon)(if self.active { p.ink } else { p.soft }, p.side)
    }
}

/// The places (Meetings, Tasks, Memory, Diary), the tools (Settings) and the filter for
/// meetings from other devices, when there are any.
fn places(s: &State, p: Palette) -> (Vec<Place>, Option<Place>) {
    let page = s.page();
    let features = &s.config.features;
    let open_tasks = s.tasks.counts.open;
    let proposals = s.memory.proposals.len();
    let high = s.check.latest.high;
    let mut places = vec![Place {
        key: "nav-meetings",
        name: "Meetings",
        help: "Meetings".into(),
        active: page == Page::Meetings && s.filter == Filter::All,
        icon: |c, _| icon_meetings(c),
        flag: None,
        press: |s| s.go_meetings(),
    }];
    if features.tasks {
        places.push(Place {
            key: "view-tasks",
            name: "Tasks",
            help: format!(
                "Tasks, {open_tasks} open, {} mine",
                s.tasks.counts.mine_open
            ),
            active: page == Page::Tasks,
            icon: |c, _| icon_tasks(c),
            flag: None,
            press: |s| {
                s.set_view(View::Tasks);
                s.nav_done();
            },
        });
    }
    if features.memory {
        places.push(Place {
            key: "view-memory",
            name: "Memory",
            help: if proposals > 0 {
                format!("Memory, {proposals} proposals to review")
            } else {
                "Memory".into()
            },
            active: page == Page::Memory,
            icon: |c, _| icon_memory(c),
            flag: (proposals > 0).then_some(p.accent),
            press: |s| {
                s.open_memory();
                s.nav_done();
            },
        });
    }
    // The Diary is read every day like the other places, so it sits with them, not in the
    // footer with the tools.
    if features.daily_check {
        places.push(Place {
            key: "view-check",
            name: "Diary",
            help: if high > 0 {
                format!("Diary, {high} open high-severity findings")
            } else {
                "Diary".into()
            },
            active: page == Page::Check,
            icon: icon_check,
            flag: (high > 0).then_some(p.warn),
            press: |s| {
                s.open_check();
                s.nav_done();
            },
        });
    }
    let peers = (features.other_devices && s.nav.peers > 0).then(|| Place {
        key: "nav-peers",
        name: "Other devices",
        help: format!(
            "Show the {} meetings recorded on another device",
            s.nav.peers
        ),
        active: page == Page::Meetings && s.filter == Filter::Peers,
        icon: |c, _| icon_device(c),
        flag: None,
        press: |s| s.set_filter(Filter::Peers),
    });
    (places, peers)
}

/// The sidebar. `wide` is the open sidebar; otherwise the rail of icons.
///
/// Three tiers: the places (Meetings, Tasks, Memory, Diary) on 40-point rows with icons; the
/// projects as lighter 32-point rows under a quiet label; one profile button in the
/// footer. Routine counts live on their pages. Two dots remain, because each waits on the user:
/// memory proposals to review, and high findings of the daily check in the Diary.
pub fn sidebar(s: &State, p: Palette, wide: bool, layer: &mut Vec<El>) -> El {
    if !wide && !s.hex_recording() {
        return el().w(0.0).h_full();
    }
    let (places, peers) = places(s, p);

    let mut header = row().w_full().h(52.0).gap(8.0).items_center().child(logo());
    header = if wide {
        header
            .px(6.0)
            .child(roles::TITLE.wrap("Sidenote", p.ink).grow(1.0))
            .child(toggle(s, p))
            .child(
                pressable(super::icons::draw(&super::icons::SEARCH, 16.0, p.soft))
                    .key("nav-search")
                    .label("Search (Ctrl+K)")
                    .padding(5.0)
                    .radius(6.0)
                    .hover(p.chip)
                    .on_press(|s: &mut State| s.toggle_finder()),
            )
    } else {
        col()
            .w_full()
            .h(88.0)
            .gap(8.0)
            .items_center()
            .pt(12.0)
            .child(logo())
            .child(toggle(s, p))
    };

    let mut top = col().w_full().gap(2.0);
    for place in places {
        let icon = place.icon(p);
        top = top.child(if wide {
            main_row(place, icon, p)
        } else {
            rail_item(place, icon, p)
        });
    }

    let middle = col().grow(1.0);

    if wide && let Some(place) = peers {
        let icon = place.icon(p);
        top = top.child(main_row(place, icon, p));
    }
    let mut footer = col()
        .w_full()
        .pt(6.0)
        .pb(8.0)
        .child(profile(s, p, wide, layer));
    if !wide && s.hex_recording() {
        footer = footer.child(
            pressable(super::icons::draw(&super::icons::MIC, 18.0, p.warn))
                .key("rail-recording")
                .label("Recording. Open live meeting")
                .help("Recording")
                .padding(8.0)
                .on_press(|s: &mut State| s.hex_line_press()),
        );
    }

    let mut pane = col()
        .key("nav")
        .w(if wide { WIDE } else { RAIL })
        .h_full()
        .min_h(0.0)
        .fill(p.side)
        .px(if wide { PAD } else { 8.0 })
        .child(header)
        .child(top)
        .child(middle)
        .child(footer);
    if !wide {
        pane = pane.items_center();
    }
    pane
}

/// The sidebar scene with every project shown.
pub fn scene_all(s: &mut State) {
    s.nav.show_all = true;
}

// ---- the top right ----------------------------------------------------------------------------

/// "New meeting", or while Hex records, "Recording 12:34 | Stop".
pub fn actions(s: &State, p: Palette, narrow: bool) -> El {
    if s.hex_recording() {
        return recording_pill(s, p, narrow);
    }
    let blocked = hex::start_blocked(s);
    let ink = if blocked.is_some() { p.soft } else { p.card };
    let b = pressable(
        row()
            .gap(8.0)
            .items_center()
            .children((!meetings_home(s)).then(|| icon_plus(ink)))
            .child(
                roles::CONTROL
                    .bold()
                    .wrap(if narrow { "New" } else { "New meeting" }, ink),
            ),
    )
    .key("new-meeting")
    .px(if narrow { 12.0 } else { 16.0 })
    .py(7.0)
    .radius(999.0)
    .fill(p.accent);
    let action = match blocked {
        None => b
            .label("New meeting")
            .help("Start recording a meeting")
            .on_press(|s: &mut State| s.new_meeting()),
        Some(reason) => b
            .label(format!("New meeting ({reason})"))
            .help(reason)
            .fill(p.chip)
            .color(p.soft)
            .focusable(false),
    };
    col().items_end().gap(2.0).child(action).children(
        blocked
            .filter(|reason| !reason.starts_with("Checking for Hex."))
            .map(|reason| {
                roles::META
                    .wrap(reason, p.soft)
                    .max_w(if narrow { 300.0 } else { 440.0 })
                    .role("status")
            }),
    )
}

fn recording_pill(s: &State, p: Palette, narrow: bool) -> El {
    let clock = hex::recording_clock(s);
    let words = match (&clock, narrow) {
        (Some(c), true) => c.clone(),
        (Some(c), false) => format!("Recording {c}"),
        (None, _) => "Recording".into(),
    };
    let open = pressable(
        row()
            .gap(8.0)
            .items_center()
            .child(dot(8.0, p.warn))
            .child(roles::CONTROL.bold().wrap(words.clone(), p.ink)),
    )
    .key("recording-open")
    .label(format!("{words}. Open the live meeting"))
    .help("Open the live meeting")
    .pl(14.0)
    .pr(12.0)
    .py(7.0)
    .radius(999.0)
    .hover(p.chip)
    .on_press(|s: &mut State| s.open_live());
    let why = hex::stop_blocked(s);
    let stop = pressable(
        row()
            .gap(7.0)
            .items_center()
            .child(icon_stop(9.0, if why.is_some() { p.faint } else { p.ink }))
            .child(
                roles::CONTROL
                    .bold()
                    .wrap("Stop", if why.is_some() { p.faint } else { p.ink }),
            ),
    )
    .key("hex-stop")
    .pl(12.0)
    .pr(14.0)
    .py(7.0)
    .radius(999.0);
    let stop = match why {
        None => stop
            .label("Stop recording")
            .help("Stop recording and write the summary")
            .hover(p.chip)
            .on_press(|s: &mut State| s.hex_record_stop()),
        Some(reason) => stop.label(format!("Stop ({reason})")).help(reason).dim(0.6),
    };
    row()
        .key("recording-pill")
        .items_center()
        .radius(999.0)
        .fill(p.card)
        .border(1.0, p.line)
        .child(open)
        .child(el().w(1.0).h(18.0).fill(p.line))
        .child(stop)
}

fn logo() -> El {
    let mut mark = el().size(20.0, 20.0);
    static PART_0: rune::Icon = rune::Icon {
        name: "sidenote-logo-0",
        view: 20.0,
        body: r#"<rect x="0" y="0" width="20" height="20" rx="4.5" fill="currentColor"/>"#,
    };
    mark = mark.child(
        rune::icon(&PART_0, 20.0)
            .color(rune::color::hex("#bcb9b0").expect("logo colour"))
            .absolute(0.0, 0.0),
    );
    static PART_1: rune::Icon = rune::Icon {
        name: "sidenote-logo-1",
        view: 20.0,
        body: r#"<rect x="1" y="1" width="18" height="18" rx="3.5" fill="currentColor"/>"#,
    };
    mark = mark.child(
        rune::icon(&PART_1, 20.0)
            .color(rune::color::hex("#1e1e1c").expect("logo colour"))
            .absolute(0.0, 0.0),
    );
    static PART_2: rune::Icon = rune::Icon {
        name: "sidenote-logo-2",
        view: 20.0,
        body: r#"<path d="M5 6 H7 V7 H5 Z M4 7 H8 V8 H4 Z M3 8 H5 V9 H3 Z M7 8 H9 V9 H7 Z M3 9 H5 V10 H3 Z M7 9 H9 V10 H7 Z M3 10 H5 V11 H3 Z M8 10 H10 V11 H8 Z M3 11 H5 V12 H3 Z M8 11 H10 V12 H8 Z M12 11 H15 V12 H12 Z M3 12 H5 V13 H3 Z M9 12 H15 V13 H9 Z M10 13 H13 V14 H10 Z M11 14 H13 V15 H11 Z" fill="currentColor"/>"#,
    };
    mark = mark.child(
        rune::icon(&PART_2, 20.0)
            .color(rune::color::hex("#fcfbf8").expect("logo colour"))
            .absolute(0.0, 0.0),
    );
    static PART_3: rune::Icon = rune::Icon {
        name: "sidenote-logo-3",
        view: 20.0,
        body: r#"<path d="M15 11 H17 V12 H15 Z M15 12 H17 V13 H15 Z" fill="currentColor"/>"#,
    };
    mark = mark.child(
        rune::icon(&PART_3, 20.0)
            .color(rune::color::hex("#e3aa55").expect("logo colour"))
            .absolute(0.0, 0.0),
    );
    mark
}
fn profile(s: &State, p: Palette, wide: bool, layer: &mut Vec<El>) -> El {
    let name = s
        .ai_notes
        .settings
        .as_ref()
        .map(|settings| settings.your_name.trim())
        .filter(|name| !name.is_empty())
        .unwrap_or("You");
    let initial = name
        .chars()
        .next()
        .unwrap_or('Y')
        .to_uppercase()
        .to_string();
    let (_, status) = hex::short_status(s, p);
    let helper_issue = s
        .ai_notes
        .settings
        .as_ref()
        .is_some_and(|settings| settings.engine == "app" && settings.helper.is_some())
        && (s.ai_notes.error.is_some()
            || s.ai_notes.test_results.values().any(|result| {
                !result.starts_with("Worked in ") && !result.starts_with("Claude Code replied")
            }));
    let status = if helper_issue && matches!(status.as_str(), "Ready" | "Checking") {
        "Meeting summary needs attention".into()
    } else {
        status
    };
    let update_ready = super::updates::footer(s, p, wide).is_some();
    let attention = update_ready
        || !matches!(
            status.as_str(),
            "Ready" | "Recording" | "Checking" | "Dictating"
        ) && !s.hex_recording();
    let avatar = || {
        let mut avatar = col()
            .size(28.0, 28.0)
            .radius(14.0)
            .fill(p.chip)
            .center()
            .child(roles::NAME.wrap(&initial, p.ink));
        if attention {
            avatar = avatar.child(dot(7.0, p.warn).absolute(21.0, 0.0));
        }
        avatar
    };
    let mut trigger = row().w_full().gap(8.0).items_center().child(avatar());
    if wide {
        trigger = trigger
            .child(roles::CONTROL.line(name, p.ink).grow(1.0))
            .child(super::icons::draw(
                &super::icons::CHEVRONS_UP_DOWN,
                16.0,
                p.soft,
            ));
    }
    let trigger = pressable(trigger.padding(6.0))
        .key("profile-button")
        .label("Profile and settings")
        .radius(8.0)
        .hover(p.chip)
        .on_press(|s: &mut State| s.nav.profile_menu = !s.nav.profile_menu);
    let sync = s.last_updated.map(|at| {
        let minutes = at.elapsed().as_secs() / 60;
        if minutes == 0 {
            "Synced just now".to_string()
        } else {
            format!("Synced {minutes} min ago")
        }
    });
    let device = std::env::var("COMPUTERNAME").unwrap_or_else(|_| "This PC".into());
    if s.nav.profile_menu {
        let device_line = format!(
            "This PC: {device}{}",
            sync.as_ref()
                .map(|status| format!(". {status}"))
                .unwrap_or_default()
        );
        let mut menu = col()
            .key("profile-menu-menu")
            .role("menu")
            .label("Profile menu")
            .focus_group()
            .w(WIDE - 2.0 * PAD)
            .padding(6.0)
            .gap(2.0)
            .fill(p.card)
            .border(1.0, p.line)
            .radius(8.0)
            .shadow(4.0, 16.0, p.shadow)
            .child(
                row()
                    .h(MAIN_H)
                    .px(PAD)
                    .gap(10.0)
                    .items_center()
                    .child(avatar())
                    .child(
                        roles::NAME
                            .line(name, p.ink)
                            .key("profile-menu-name")
                            .grow(1.0),
                    ),
            )
            .child(
                roles::SMALL
                    .wrap(device_line, p.soft)
                    .max_lines(2)
                    .key("profile-menu-device")
                    .px(PAD)
                    .pb(8.0),
            );
        if update_ready {
            menu = menu.child(profile_item(
                "Update ready",
                "profile-update",
                &super::icons::UPDATES,
                p,
                |s| {
                    s.nav.profile_menu = false;
                    super::updates::settings_scene(s);
                },
            ));
        }
        if attention && !update_ready {
            menu = menu.child(profile_item(
                &format!("{status}. Fix"),
                "profile-fix",
                &super::icons::WARNING,
                p,
                move |s| {
                    s.nav.profile_menu = false;
                    if helper_issue {
                        s.open_ai_notes();
                    } else {
                        s.open_hex();
                    }
                },
            ));
        }
        menu = menu
            .child(profile_item(
                "Prompt library",
                "profile-menu-item2",
                &super::icons::PROMPTS,
                p,
                |s| {
                    s.nav.profile_menu = false;
                    s.open_ai_notes();
                    super::scene_scroll(s, "agent-instructions");
                },
            ))
            .child(profile_item(
                "Help",
                "profile-menu-item3",
                &super::icons::HELP,
                p,
                |s| {
                    s.nav.profile_menu = false;
                    s.open_hex();
                    s.hex.section = super::hex::Section::Shortcuts;
                },
            ))
            .child(
                profile_item(
                    "Settings",
                    "profile-menu-item4",
                    &super::icons::SETTINGS,
                    p,
                    |s| {
                        s.nav.profile_menu = false;
                        s.open_hex();
                    },
                )
                .child(roles::META.line("Ctrl+,", p.soft)),
            );
        layer.push(rune_kit::scrim("profile-menu-scrim", |s: &mut State| {
            s.nav.profile_menu = false
        }));
        layer.push(
            menu.anchored_fit("profile-button", rune::Side::Top, rune::Align::Start, 6.0)
                .focus_return("profile-button")
                .modal(true, |s: &mut State| s.nav.profile_menu = false),
        );
    }
    trigger
}

fn profile_item(
    name: &str,
    key: &str,
    icon: &'static rune::Icon,
    p: Palette,
    action: impl Fn(&mut State) + 'static,
) -> El {
    pressable(
        row()
            .w_full()
            .h(MAIN_H)
            .px(PAD)
            .gap(10.0)
            .items_center()
            .child(super::icons::draw(icon, 18.0, p.soft))
            .child(roles::CONTROL.line(name, p.soft).grow(1.0)),
    )
    .key(key.to_string())
    .role("menuitem")
    .label(name.to_string())
    .help(name.to_string())
    .radius(6.0)
    .hover(p.chip)
    .focus_ring(1.0, 0.0, p.accent)
    .on_press(action)
}

pub fn meetings_home(s: &State) -> bool {
    s.page() == Page::Meetings
        && s.selected.is_none()
        && !s.on_live()
        && !matches!(s.filter, Filter::Area(_) | Filter::Areas)
}

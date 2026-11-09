//! The left sidebar and the top-right actions.
//!
//! The sidebar holds three places (Meetings, Tasks, Memory), the most recent projects as
//! filters for the meeting list, and a footer with Check, Settings and a compact Hex status
//! line. It folds to a rail of icons.
//! The top right holds one primary "New meeting" button, which becomes "Recording 12:34 | Stop"
//! while Hex records, so the owner can get back to the live page or stop from anywhere.
//! All icons are drawn here from plain shapes.
use super::{
    State, hex, roles,
    tasks::View,
    theme::Palette,
    widgets::{dot, hairline, pressable, scroll, sidebar_icon},
};
use chrono::{DateTime, Local};
use rune::{Color, El, TextStyle, col, el, row, text};
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
        let mut items: HashMap<&str, Vec<&core::TaskItem>> = HashMap::new();
        for t in self.tasks.all.iter().filter(|t| !t.manual) {
            items.entry(t.meeting_id.as_str()).or_default().push(t);
        }
        let mut cache = NavCache {
            show_all: self.nav.show_all,
            ..NavCache::default()
        };
        for m in &self.meetings {
            if m.device.is_some() {
                cache.peers += 1;
            }
            let project = items
                .get(m.id.as_str())
                .and_then(|v| core::dominant_project(v.iter().copied()));
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
            Filter::Peers => meeting.device.is_some(),
        }
    }

    pub fn set_filter(&mut self, filter: Filter) {
        if matches!(filter, Filter::Project(_)) && !self.config.features.projects
            || filter == Filter::Peers && !self.config.features.other_devices
        {
            return;
        }
        self.go_home();
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
            let prefs = UiPrefs {
                sidebar_collapsed: Some(!open),
            };
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
        RAIL + 1.0
    } else {
        WIDE + 1.0
    }
}

// ---- icons ------------------------------------------------------------------------------------

fn glyph(g: &str, size: f32, weight: f32, ink: Color) -> El {
    text(g.to_string())
        .font(TextStyle::new(size, weight).lh(size + 2.0))
        .color(ink)
}

fn slot() -> El {
    col().size(20.0, 20.0).center()
}

/// A page with three lines.
fn icon_meetings(c: Color) -> El {
    let rule = |w: f32| el().w(w).h(1.5).radius(1.0).fill(c);
    slot().child(
        col()
            .size(14.0, 17.0)
            .radius(3.0)
            .border(1.5, c)
            .px(2.5)
            .pt(4.0)
            .gap(2.0)
            .child(rule(6.0))
            .child(rule(6.0))
            .child(rule(4.0)),
    )
}

/// A box with a tick in it.
fn icon_tasks(c: Color) -> El {
    slot().child(
        col()
            .size(16.0, 16.0)
            .radius(4.5)
            .border(1.5, c)
            .center()
            .child(glyph("\u{2713}", 10.0, 700.0, c)),
    )
}

/// A ring with a dot in the middle.
fn icon_memory(c: Color) -> El {
    slot().child(
        col()
            .size(17.0, 17.0)
            .radius(9.0)
            .border(1.5, c)
            .center()
            .child(el().size(5.0, 5.0).radius(3.0).fill(c)),
    )
}

/// A calendar page: two pegs, a header rule and a day.
fn icon_check(c: Color, _bg: Color) -> El {
    slot().child(
        el().size(16.0, 18.0)
            .child(el().absolute(3.5, 0.0).size(2.0, 4.5).radius(1.0).fill(c))
            .child(el().absolute(10.5, 0.0).size(2.0, 4.5).radius(1.0).fill(c))
            .child(
                el().absolute(0.0, 2.0)
                    .size(16.0, 15.0)
                    .radius(3.5)
                    .border(1.5, c),
            )
            .child(el().absolute(1.5, 6.5).size(13.0, 1.5).fill(c))
            .child(el().absolute(5.5, 10.0).size(5.0, 4.0).radius(1.0).fill(c)),
    )
}

/// Three sliders.
fn icon_hex(c: Color, bg: Color) -> El {
    let bar = |knob: f32| {
        el().size(17.0, 5.0)
            .child(el().absolute(0.0, 1.75).size(17.0, 1.5).radius(1.0).fill(c))
            .child(
                el().absolute(knob, 0.0)
                    .size(5.0, 5.0)
                    .radius(3.0)
                    .border(1.5, c)
                    .fill(bg),
            )
    };
    slot().child(
        col()
            .gap(3.0)
            .child(bar(10.0))
            .child(bar(2.0))
            .child(bar(7.0)),
    )
}

/// A folder.
fn icon_folder(c: Color) -> El {
    slot().child(
        el().size(16.0, 13.0)
            .child(el().absolute(0.0, 0.0).size(7.0, 4.0).radius(2.0).fill(c))
            .child(
                el().absolute(0.0, 2.5)
                    .size(16.0, 10.5)
                    .radius(3.0)
                    .border(1.5, c),
            ),
    )
}

/// A screen on a stand.
fn icon_device(c: Color) -> El {
    slot().child(
        el().size(16.0, 15.0)
            .child(
                el().absolute(0.0, 0.0)
                    .size(16.0, 11.0)
                    .radius(2.5)
                    .border(1.5, c),
            )
            .child(el().absolute(4.0, 13.0).size(8.0, 1.5).radius(1.0).fill(c)),
    )
}

pub fn icon_plus(c: Color) -> El {
    el().size(13.0, 13.0)
        .child(el().absolute(0.0, 5.5).size(13.0, 2.0).radius(1.0).fill(c))
        .child(el().absolute(5.5, 0.0).size(2.0, 13.0).radius(1.0).fill(c))
}

/// The mark of Stop: a rounded square.
pub fn icon_stop(size: f32, c: Color) -> El {
    el().size(size, size).radius(size * 0.22).fill(c)
}

// ---- the sidebar ------------------------------------------------------------------------------

/// Row heights: the three places, the tools in the footer, and a project under its label.
const MAIN_H: f32 = 40.0;
const TOOL_H: f32 = 36.0;
const FOLDER_H: f32 = 32.0;
/// The open sidebar's side padding, and how far a project sits in from its label.
const PAD: f32 = 10.0;
const FOLDER_INDENT: f32 = 12.0;
/// Check and Settings share one footer line.
const TOOL_W: f32 = (WIDE - 2.0 * PAD - 4.0) / 2.0;
/// Projects shown before "Show all".
const RECENT: usize = 6;

/// The one quiet mark for something that waits on the user: memory proposals to review, high
/// check findings. Counts live on those pages.
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

/// One of the three places in the open sidebar: an icon and a name on a 40-point row.
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

/// A tool in the footer (Check, Settings, other devices): shorter than a place, same type.
fn tool_row(place: Place, icon: El, p: Palette) -> El {
    let mut r = row()
        .h(TOOL_H)
        .gap(8.0)
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
        .label(place.help.clone())
        .help(place.help)
        .radius(8.0)
        .fill(bg)
        .hover(hover)
        .on_press(place.press)
}

/// A project: no icon and no count, lighter than a place, indented under its label.
fn folder_row(key: String, name: &str, active: bool, p: Palette) -> El {
    let words = if active {
        roles::SMALL.medium().line(name, p.ink)
    } else {
        roles::SMALL.line(name, p.soft)
    };
    let (bg, hover) = fills(active, p);
    pressable(
        row()
            .w_full()
            .h(FOLDER_H)
            .items_center()
            .pl(PAD + FOLDER_INDENT)
            .pr(PAD)
            .child(words.grow(1.0)),
    )
    .key(key)
    .label(format!("Show meetings in {name}"))
    .radius(6.0)
    .fill(bg)
    .hover(hover)
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
        "Hide the sidebar (Ctrl+B)"
    } else {
        "Show the sidebar (Ctrl+B)"
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

fn hex_line(s: &State, p: Palette, wide: bool) -> El {
    let (ink, words) = hex::short_status(s, p);
    let recording = s.hex_recording();
    let help = if recording {
        format!("Hex: {words}. Open the live meeting")
    } else {
        format!("Hex: {words}. Open Hex settings")
    };
    let on_press = |s: &mut State| s.hex_line_press();
    if wide {
        pressable(
            row()
                .w_full()
                .gap(8.0)
                .items_center()
                .px(PAD)
                .py(8.0)
                .child(dot(8.0, ink))
                .child(roles::META.wrap("Hex", p.faint))
                .child(roles::CONTROL.line(words, p.ink).grow(1.0)),
        )
        .key("hex-status")
        .label(help.clone())
        .help(help)
        .radius(8.0)
        .hover(p.chip)
        .on_press(on_press)
    } else {
        pressable(col().w(40.0).h(34.0).center().child(dot(9.0, ink)))
            .key("hex-status")
            .label(help.clone())
            .help(help)
            .radius(8.0)
            .hover(p.chip)
            .on_press(on_press)
    }
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

/// The places (Meetings, Tasks, Memory), the tools (Check, Settings) and the filter for
/// meetings from other devices, when there are any.
fn places(s: &State, p: Palette) -> (Vec<Place>, Vec<Place>, Option<Place>) {
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
    let mut tools = Vec::new();
    if features.daily_check {
        tools.push(Place {
            key: "view-check",
            name: "Check",
            help: if high > 0 {
                format!("Daily check, {high} open high-severity findings")
            } else {
                "Daily check".into()
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
    tools.push(Place {
        key: "view-hex",
        name: "Settings",
        help: "Settings".into(),
        active: page == Page::Hex,
        icon: icon_hex,
        flag: None,
        press: |s| {
            s.open_hex();
            s.nav_done();
        },
    });
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
    (places, tools, peers)
}

/// The sidebar. `wide` is the open sidebar; otherwise the rail of icons.
///
/// Three tiers: the places (Meetings, Tasks, Memory) on 40-point rows with icons; the projects
/// as lighter 32-point rows under a quiet label; the tools (Check, Settings) and Hex's status in
/// the footer. Routine counts live on their pages. Two dots remain, because each waits on the
/// user: memory proposals to review, and high findings in the daily check.
pub fn sidebar(s: &State, p: Palette, wide: bool) -> El {
    let (places, tools, peers) = places(s, p);

    let mut header = row().w_full().h(52.0).items_center().child(toggle(s, p));
    header = if wide {
        header.px(6.0).child(
            roles::TITLE
                .wrap("Sidenote", p.ink)
                .help("Local-first meeting notes on top of Hex"),
        )
    } else {
        header.justify_center()
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

    let projects_on = s.config.features.projects;
    let middle = if wide && projects_on {
        projects(s, p)
    } else if !wide && (projects_on || peers.is_some()) {
        // The rail has no room for names, so one folder opens the sidebar on them.
        let more = Place {
            key: "nav-projects",
            name: "Projects",
            help: if projects_on {
                "Projects. Open the sidebar"
            } else {
                "Other devices. Open the sidebar"
            }
            .into(),
            active: matches!(s.filter, Filter::Project(_) | Filter::Peers)
                && s.page() == Page::Meetings,
            icon: |c, _| icon_folder(c),
            flag: None,
            press: |s| s.set_sidebar(true),
        };
        let icon = icon_folder(p.soft);
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .items_center()
            .child(hairline(p).my(10.0))
            .child(rail_item(more, icon, p))
    } else {
        col().grow(1.0)
    };

    let mut footer = col().w_full().gap(2.0).pt(6.0).pb(8.0);
    if !wide {
        footer = footer.items_center();
    }
    if let Some(notice) = super::updates::footer(s, p, wide) {
        footer = footer.child(if wide { notice.px(PAD) } else { notice });
    }
    if wide {
        if let Some(place) = peers {
            let icon = place.icon(p);
            footer = footer.child(tool_row(place, icon, p).w_full());
        }
        let mut line = row().w_full().gap(4.0);
        for place in tools {
            let icon = place.icon(p);
            line = line.child(tool_row(place, icon, p).w(TOOL_W));
        }
        footer = footer.child(line);
    } else {
        for place in tools {
            let icon = place.icon(p);
            footer = footer.child(rail_item(place, icon, p));
        }
    }
    footer = footer.child(hex_line(s, p, wide));

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
        .child(hairline(p))
        .child(footer);
    if !wide {
        pane = pane.items_center();
    }
    pane
}

impl State {
    /// Projects, most recently met first: by each one's newest meeting, then by name.
    fn recent_projects(&self) -> Vec<&String> {
        let mut names: Vec<&String> = self.tasks.names.iter().collect();
        names.sort_by(|a, b| {
            let newest = |n: &String| self.nav.project_latest.get(n).copied();
            newest(b)
                .cmp(&newest(a))
                .then_with(|| a.to_lowercase().cmp(&b.to_lowercase()))
        });
        names
    }
}

/// The Projects section: a quiet label, then the six most recently active projects as filters
/// for the meeting list, and "Show all" for the rest. The choice lasts for the session.
fn projects(s: &State, p: Palette) -> El {
    let chosen = match &s.filter {
        Filter::Project(name) if s.page() == Page::Meetings => Some(name),
        _ => None,
    };
    let mut list = col().w_full().gap(2.0).pb(8.0).child(
        roles::GROUP
            .wrap("Projects", p.soft)
            .px(PAD)
            .mt(roles::GROUP_TOP)
            .mb(4.0),
    );
    let names = s.recent_projects();
    if names.is_empty() {
        list = list.child(
            roles::META
                .wrap("Projects you add meetings to appear here.", p.faint)
                .pl(PAD + FOLDER_INDENT)
                .pr(PAD),
        );
    }
    let all = s.nav.show_all || names.len() <= RECENT;
    for (i, name) in names.iter().enumerate() {
        let active = chosen == Some(*name);
        // A chosen project stays in sight even when it is not among the recent ones.
        if !all && i >= RECENT && !active {
            continue;
        }
        let target = (*name).clone();
        list = list.child(
            folder_row(format!("nav-project-{i}"), name, active, p)
                .on_press(move |s: &mut State| s.set_filter(Filter::Project(target.clone()))),
        );
    }
    if names.len() > RECENT {
        let (words, help) = if s.nav.show_all {
            (
                "Show fewer",
                "Show only the six most recent projects".to_string(),
            )
        } else {
            ("Show all", format!("Show all {} projects", names.len()))
        };
        list = list.child(
            pressable(
                row()
                    .w_full()
                    .h(FOLDER_H)
                    .items_center()
                    .pl(PAD + FOLDER_INDENT)
                    .pr(PAD)
                    .child(roles::SMALL.line(words, p.faint)),
            )
            .key("nav-projects-all")
            .label(help)
            .radius(6.0)
            .hover(p.chip)
            .on_press(|s: &mut State| s.nav.show_all = !s.nav.show_all),
        );
    }
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(scroll(list, "nav-scroll"))
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
    let ink = p.card;
    let b = pressable(
        row().gap(8.0).items_center().child(icon_plus(ink)).child(
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
    match blocked {
        None => b
            .label("New meeting")
            .help("Start recording a meeting")
            .on_press(|s: &mut State| s.new_meeting()),
        Some(reason) => b
            .label(format!("New meeting ({reason})"))
            .help(reason)
            .dim(0.5),
    }
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
            .help("Stop recording and make notes")
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

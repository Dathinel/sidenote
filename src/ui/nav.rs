//! The left sidebar and the top-right actions.
//!
//! The sidebar holds the five places (Meetings, Tasks, Memory, Check, Hex), the projects as
//! filters for the meeting list, and a compact Hex status line. It folds to a rail of icons.
//! The top right holds one primary "New meeting" button, which becomes "Recording 12:34 | Stop"
//! while Hex records, so the owner can get back to the live page or stop from anywhere.
//! All icons are drawn here from plain shapes.
use super::{
    State, hex,
    tasks::View,
    theme::Palette,
    widgets::{dot, hairline, label, line, pressable, scroll, sidebar_icon},
};
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

/// Looked up once per rebuild, not per frame: each meeting's project and the counts the
/// sidebar shows beside the filters.
#[derive(Default)]
pub struct NavCache {
    pub meeting_project: HashMap<String, String>,
    pub project_counts: HashMap<String, usize>,
    pub peers: usize,
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
        let mut cache = NavCache::default();
        for m in &self.meetings {
            if m.device.is_some() {
                cache.peers += 1;
            }
            let project = items
                .get(m.id.as_str())
                .and_then(|v| core::dominant_project(v.iter().copied()));
            if let Some(name) = project {
                *cache.project_counts.entry(name.clone()).or_default() += 1;
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

fn pill(n: usize, ink: Color, bg: Color) -> El {
    label(n.to_string(), 12.0, 700.0, ink)
        .px(7.0)
        .radius(999.0)
        .fill(bg)
}

/// One of the five places, or a filter, in the open sidebar.
fn row_item(
    key: String,
    icon: El,
    name: &str,
    tail: Option<El>,
    active: bool,
    help: String,
    p: Palette,
) -> El {
    let mut r = row()
        .w_full()
        .gap(10.0)
        .items_center()
        .px(10.0)
        .py(6.0)
        .radius(8.0)
        .child(icon)
        .child(
            line(
                name,
                14.0,
                if active { 600.0 } else { 500.0 },
                if active { p.ink } else { p.soft },
            )
            .grow(1.0),
        );
    if let Some(t) = tail {
        r = r.child(t);
    }
    pressable(r)
        .key(key)
        .label(help)
        .fill(if active { p.chip } else { p.side })
        .hover(p.chip)
}

/// The same in the rail: the icon alone, with a dot where the open row has a count.
fn rail_item(
    key: String,
    icon: El,
    flag: Option<Color>,
    active: bool,
    help: String,
    p: Palette,
) -> El {
    let mut b = col().w(40.0).h(36.0).center().child(icon);
    if let Some(c) = flag {
        b = b.child(dot(8.0, c).absolute(24.0, 5.0).border(1.5, p.side));
    }
    pressable(b)
        .key(key)
        .label(help.clone())
        .help(help)
        .radius(8.0)
        .fill(if active { p.chip } else { p.side })
        .hover(p.chip)
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
                .px(10.0)
                .py(8.0)
                .child(dot(8.0, ink))
                .child(label("Hex", 13.0, 500.0, p.faint))
                .child(line(words, 13.0, 600.0, p.ink).grow(1.0)),
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

fn section_title(text_: &str, p: Palette) -> El {
    label(text_, 12.0, 600.0, p.faint).px(10.0).mt(16.0).mb(4.0)
}

/// The sidebar. `wide` is the open sidebar; otherwise the rail of icons.
pub fn sidebar(s: &State, p: Palette, wide: bool) -> El {
    let page = s.page();
    let on_meetings = page == Page::Meetings && s.filter == Filter::All;
    let c = |on: bool| if on { p.ink } else { p.soft };
    let open_tasks = s.tasks.counts.open;
    let proposals = s.memory.proposals.len();
    let high = s.check.latest.high;

    struct Spec {
        key: &'static str,
        name: &'static str,
        help: String,
        active: bool,
        icon: fn(Color, Color) -> El,
        count: Option<El>,
        flag: Option<Color>,
        press: fn(&mut State),
    }
    let specs = [
        Spec {
            key: "nav-meetings",
            name: "Meetings",
            help: "Meetings".into(),
            active: on_meetings,
            icon: |c, _| icon_meetings(c),
            count: None,
            flag: None,
            press: |s| s.go_meetings(),
        },
        Spec {
            key: "view-tasks",
            name: "Tasks",
            help: format!(
                "Tasks, {open_tasks} open, {} mine",
                s.tasks.counts.mine_open
            ),
            active: page == Page::Tasks,
            icon: |c, _| icon_tasks(c),
            count: (open_tasks > 0).then(|| label(open_tasks.to_string(), 12.5, 500.0, p.faint)),
            flag: None,
            press: |s| {
                s.set_view(View::Tasks);
                s.nav_done();
            },
        },
        Spec {
            key: "view-memory",
            name: "Memory",
            help: if proposals > 0 {
                format!("Memory, {proposals} proposals to review")
            } else {
                "Memory".into()
            },
            active: page == Page::Memory,
            icon: |c, _| icon_memory(c),
            count: (proposals > 0).then(|| pill(proposals, p.card, p.accent)),
            flag: (proposals > 0).then_some(p.accent),
            press: |s| {
                s.open_memory();
                s.nav_done();
            },
        },
        Spec {
            key: "view-check",
            name: "Check",
            help: if high > 0 {
                format!("Daily check, {high} open high-severity findings")
            } else {
                "Daily check".into()
            },
            active: page == Page::Check,
            icon: icon_check,
            count: (high > 0).then(|| pill(high, p.warn, p.warnbg)),
            flag: (high > 0).then_some(p.warn),
            press: |s| {
                s.open_check();
                s.nav_done();
            },
        },
        Spec {
            key: "view-hex",
            name: "Settings",
            help: "Settings".into(),
            active: page == Page::Hex,
            icon: icon_hex,
            count: None,
            flag: None,
            press: |s| {
                s.open_hex();
                s.nav_done();
            },
        },
    ];

    let mut top = col().w_full().gap(2.0);
    for spec in specs {
        if spec.key == "view-tasks" && !s.config.features.tasks
            || spec.key == "view-memory" && !s.config.features.memory
            || spec.key == "view-check" && !s.config.features.daily_check
        {
            continue;
        }
        let ink = c(spec.active);
        let icon = (spec.icon)(ink, p.side);
        top = top.child(if wide {
            row_item(
                spec.key.into(),
                icon,
                spec.name,
                spec.count,
                spec.active,
                spec.help,
                p,
            )
            .on_press(spec.press)
        } else {
            rail_item(spec.key.into(), icon, spec.flag, spec.active, spec.help, p)
                .on_press(spec.press)
        });
    }

    let mut header = row().w_full().h(52.0).items_center().child(toggle(s, p));
    header = if wide {
        header.px(6.0).child(
            label("Sidenote", 16.0, 600.0, p.ink).help("Local-first meeting notes on top of Hex"),
        )
    } else {
        header.justify_center()
    };

    let middle = if !s.config.features.projects && !s.config.features.other_devices {
        col().grow(1.0)
    } else if wide {
        projects(s, p)
    } else {
        // The rail has no room for names, so one folder opens the sidebar on them.
        let more = rail_item(
            "nav-projects".into(),
            icon_folder(p.soft),
            None,
            matches!(s.filter, Filter::Project(_) | Filter::Peers) && page == Page::Meetings,
            if s.config.features.projects {
                "Projects. Open the sidebar"
            } else {
                "Other devices. Open the sidebar"
            }
            .into(),
            p,
        )
        .on_press(|s: &mut State| s.set_sidebar(true));
        col()
            .w_full()
            .grow(1.0)
            .min_h(0.0)
            .child(hairline(p).my(10.0))
            .child(more)
    };

    let status = hex_line(s, p, wide);
    let mut pane = col()
        .key("nav")
        .w(if wide { WIDE } else { RAIL })
        .h_full()
        .min_h(0.0)
        .fill(p.side)
        .px(if wide { 10.0 } else { 8.0 })
        .child(header)
        .child(top)
        .child(middle)
        .child(hairline(p))
        .child(col().w_full().py(8.0).child(status));
    if !wide {
        pane = pane.items_center();
    }
    pane
}

/// The Projects section: every project as a filter for the meeting list, preceded by the
/// meetings that came from another device, when there are any.
fn projects(s: &State, p: Palette) -> El {
    let page = s.page();
    let mut list = col().w_full().gap(2.0).pb(8.0);
    // Devices come first: a long project list would push this one out of sight.
    if s.config.features.other_devices && s.nav.peers > 0 {
        let active = page == Page::Meetings && s.filter == Filter::Peers;
        list = list.child(section_title("Devices", p)).child(
            row_item(
                "nav-peers".into(),
                icon_device(if active { p.ink } else { p.soft }),
                "From other devices",
                Some(label(s.nav.peers.to_string(), 12.5, 400.0, p.faint)),
                active,
                "Show meetings recorded on another device".into(),
                p,
            )
            .on_press(|s: &mut State| s.set_filter(Filter::Peers)),
        );
    }
    if s.config.features.projects {
        list = list.child(section_title("Projects", p));
        if s.tasks.names.is_empty() {
            list = list.child(
                label(
                    "Projects you add meetings to appear here.",
                    12.5,
                    400.0,
                    p.faint,
                )
                .px(10.0),
            );
        }
        for (i, name) in s.tasks.names.iter().enumerate() {
            let active = page == Page::Meetings && s.filter == Filter::Project(name.clone());
            let n = s.nav.project_counts.get(name).copied().unwrap_or(0);
            let target = name.clone();
            list = list.child(
                row_item(
                    format!("nav-project-{i}"),
                    icon_folder(if active { p.ink } else { p.soft }),
                    name,
                    (n > 0).then(|| label(n.to_string(), 12.5, 400.0, p.faint)),
                    active,
                    format!("Show meetings in {name}"),
                    p,
                )
                .on_press(move |s: &mut State| s.set_filter(Filter::Project(target.clone()))),
            );
        }
    }
    col()
        .w_full()
        .grow(1.0)
        .min_h(0.0)
        .child(scroll(list, "nav-scroll"))
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
        row()
            .gap(8.0)
            .items_center()
            .child(icon_plus(ink))
            .child(label(
                if narrow { "New" } else { "New meeting" },
                13.5,
                600.0,
                ink,
            )),
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
            .child(label(words.clone(), 13.5, 600.0, p.ink)),
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
            .child(label(
                "Stop",
                13.5,
                600.0,
                if why.is_some() { p.faint } else { p.ink },
            )),
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

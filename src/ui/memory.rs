//! The Memory page: what the company memory holds, one claim per row, with the proposals the
//! daily check wants approved and, until the owner accepts it, the migration of the old text.
//! Every change here is the owner's own action; the daily check only proposes.
use super::{
    State,
    check::{copy_row, prose, source_row},
    list::text_title,
    roles,
    tasks::{button, pill, prop_row},
    theme::Palette,
    widgets::{
        badge, dot, hairline, label, list_and_panel, notice_bar, panel_frame, panel_top, pressable,
        scroll, spaced_caps, split_geometry,
    },
};
use chrono::{DateTime, Local, NaiveDate};
use rune::{El, Key, KeyEvent, TextEdit, Tx, col, el, row};
use rune_kit::{Kit, Menu, MenuItem, Placement, SegmentedControl, TextArea, TextInput};
use sidenote::core::memory::{
    Basis, MemoryEvent, MemoryRecord, MemorySource, MemoryStore, MigrationPreview, PendingProposal,
    ProposalOp, RecordStatus, RecordType,
};
use std::collections::HashMap;

/// A claim not confirmed for this many days is suggested for review. Information only: age
/// never changes a claim.
const REVIEW_DAYS: i64 = 90;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Review,
    All,
    Kind(RecordType),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Active,
    NeedsReview,
    Retired,
}

/// Before migration: the old text, or the claims that would be imported.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Before {
    Text,
    Candidates,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    Retire,
    Forget,
}

pub struct MemState {
    pub projects: sidenote::core::Projects,
    pub records: Vec<MemoryRecord>,
    pub proposals: Vec<PendingProposal>,
    pub events: Vec<MemoryEvent>,
    pub migrated: bool,
    pub accepted_at: Option<String>,
    pub preview: Option<MigrationPreview>,
    pub preview_running: bool,
    pub legacy: String,
    pub tab: Tab,
    pub scope: Scope,
    pub before: Before,
    pub search: TextEdit,
    pub project: Option<String>,
    pub project_menu: bool,
    pub cursor: Option<String>,
    pub keyboard: bool,
    pub panel: bool,
    pub editing: bool,
    pub edit: TextEdit,
    pub ask: Option<Ask>,
    /// Why an action failed, by record, proposal or `migration`.
    pub errors: HashMap<String, String>,
    pub notice: Option<String>,
    pub today: NaiveDate,
}

impl Default for MemState {
    fn default() -> Self {
        Self {
            projects: sidenote::core::Projects::default(),
            records: Vec::new(),
            proposals: Vec::new(),
            events: Vec::new(),
            migrated: false,
            accepted_at: None,
            preview: None,
            preview_running: false,
            legacy: String::new(),
            tab: Tab::All,
            scope: Scope::Active,
            before: Before::Text,
            search: TextEdit::default(),
            project: None,
            project_menu: false,
            cursor: None,
            keyboard: false,
            panel: false,
            editing: false,
            edit: TextEdit::default(),
            ask: None,
            errors: HashMap::new(),
            notice: None,
            today: Local::now().date_naive(),
        }
    }
}

fn parse_day(at: &str) -> Option<NaiveDate> {
    DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|t| t.with_timezone(&Local).date_naive())
}

fn needs_review(r: &MemoryRecord, today: NaiveDate) -> bool {
    match r.status {
        RecordStatus::Proposed => true,
        RecordStatus::Active => r
            .last_confirmed
            .as_deref()
            .and_then(parse_day)
            .is_none_or(|d| (today - d).num_days() > REVIEW_DAYS),
        _ => false,
    }
}

/// A claim's text for a one-line row: no citation markup, no emphasis marks.
fn plain(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find("[[") {
        out.push_str(&rest[..start]);
        match rest[start..].find("]]") {
            Some(end) => rest = &rest[start + end + 2..],
            None => {
                rest = "";
                break;
            }
        }
    }
    out.push_str(rest);
    let out = out.replace(['*', '`'], "");
    let first = out
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    let mut s = String::new();
    for w in first.split_whitespace() {
        if !s.is_empty() {
            s.push(' ');
        }
        s.push_str(w);
    }
    s
}

fn kind_label(t: RecordType) -> &'static str {
    match t {
        RecordType::Person => "People",
        RecordType::Project => "Projects",
        RecordType::Decision => "Decisions",
        RecordType::Commitment => "Commitments",
        RecordType::Question => "Open questions",
        RecordType::Story => "Story",
    }
}

fn kind_one(t: RecordType) -> &'static str {
    match t {
        RecordType::Person => "Person",
        RecordType::Project => "Project",
        RecordType::Decision => "Decision",
        RecordType::Commitment => "Commitment",
        RecordType::Question => "Open question",
        RecordType::Story => "Story",
    }
}

const KINDS: [RecordType; 6] = [
    RecordType::Person,
    RecordType::Project,
    RecordType::Decision,
    RecordType::Commitment,
    RecordType::Question,
    RecordType::Story,
];

fn basis_name(b: Basis) -> &'static str {
    match b {
        Basis::OwnerConfirmed => "You confirmed it",
        Basis::SourceSupported => "Supported by a source",
        Basis::Inferred => "Inferred, not stated",
        Basis::Unknown => "Not known",
    }
}

fn status_name(s: RecordStatus) -> &'static str {
    match s {
        RecordStatus::Proposed => "Proposed",
        RecordStatus::Active => "Active",
        RecordStatus::Superseded => "Replaced",
        RecordStatus::Retired => "Retired",
        RecordStatus::Forgotten => "Forgotten",
    }
}

fn op_name(op: ProposalOp) -> &'static str {
    match op {
        ProposalOp::Add => "Add",
        ProposalOp::Confirm => "Confirm",
        ProposalOp::Supersede => "Replace",
        ProposalOp::Retire => "Retire",
    }
}

fn day_text(at: &str) -> String {
    parse_day(at).map_or_else(|| at.to_string(), |d| d.format("%-d %b %Y").to_string())
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn confirmed_text(r: &MemoryRecord) -> String {
    match r.last_confirmed.as_deref().and_then(parse_day) {
        Some(d) => format!("Confirmed {}", d.format("%-d %b")),
        None => "Not confirmed".to_string(),
    }
}

// ---- state ---------------------------------------------------------------------------------

impl MemState {
    /// Reads the records and the pending proposals. `full` also asks the store to recover
    /// proposals from saved reports, which writes; the periodic rescan reads the merged device view.
    pub fn refresh(&mut self, full: bool) {
        self.today = Local::now().date_naive();
        self.notice = None;
        let store = MemoryStore::default();
        if let Ok(projects) = sidenote::core::TaskStore::default().load_projects() {
            self.projects = projects;
            self.project = self.project.as_deref().map(|n| self.projects.resolve(n));
        }
        if full && let Err(e) = store.list_pending_proposals() {
            self.notice = Some(format!("Could not read the memory proposals. {e}."));
        }
        match store.load() {
            Ok(file) => {
                self.migrated = file.migration.is_some();
                self.accepted_at = file.migration.as_ref().map(|m| m.accepted_at.clone());
                self.records = file.records;
                self.events = file.events;
                self.proposals = file
                    .proposals
                    .into_iter()
                    .filter(|p| p.status == sidenote::core::memory::ProposalStatus::Pending)
                    .collect();
            }
            Err(e) => {
                self.notice = Some(format!("Could not read the company memory. {e}."));
            }
        }
        if self.migrated {
            self.preview = None;
            self.legacy = String::new();
        } else {
            self.preview = store.preview_migration().ok();
            self.legacy = store.company_memory().unwrap_or_default();
            self.tab = Tab::All;
        }
        if self.cursor.as_ref().is_some_and(|c| !self.has_target(c)) {
            self.cursor = None;
            self.panel = false;
            self.editing = false;
        }
    }

    pub fn typing(&self) -> bool {
        self.search.focused || self.edit.focused
    }

    fn has_target(&self, id: &str) -> bool {
        self.records.iter().any(|r| r.id == id)
            || self.proposals.iter().any(|p| p.proposal.id == id)
            || self
                .preview
                .as_ref()
                .is_some_and(|v| v.candidates.iter().any(|c| c.record.id == id))
    }

    fn record(&self, id: &str) -> Option<&MemoryRecord> {
        self.records.iter().find(|r| r.id == id)
    }

    fn shown_record(&self, r: &MemoryRecord) -> bool {
        let text = r.text.as_deref().unwrap_or_default();
        r.status != RecordStatus::Forgotten
            && !text.is_empty()
            && match self.tab {
                Tab::Kind(t) => r.record_type == t,
                _ => true,
            }
            && match self.scope {
                Scope::Active => r.status == RecordStatus::Active,
                Scope::NeedsReview => needs_review(r, self.today),
                Scope::Retired => {
                    matches!(r.status, RecordStatus::Retired | RecordStatus::Superseded)
                }
            }
            && self
                .project
                .as_ref()
                .is_none_or(|p| r.project.as_deref() == Some(p.as_str()))
            && {
                let q = self.search.text.trim().to_lowercase();
                q.is_empty()
                    || text.to_lowercase().contains(&q)
                    || r.project
                        .as_deref()
                        .is_some_and(|p| self.projects.project_matches_query(p, &q))
                    || r.people.iter().any(|p| p.to_lowercase().contains(&q))
            }
    }

    /// The ids of the rows on screen, in order.
    pub fn shown_ids(&self) -> Vec<String> {
        if !self.migrated {
            return match (self.before, &self.preview) {
                (Before::Candidates, Some(v)) => {
                    v.candidates.iter().map(|c| c.record.id.clone()).collect()
                }
                _ => Vec::new(),
            };
        }
        if self.tab == Tab::Review {
            return self
                .proposals
                .iter()
                .map(|p| p.proposal.id.clone())
                .collect();
        }
        self.sorted_records().iter().map(|r| r.id.clone()).collect()
    }

    /// Rows grouped by kind of claim, each group in the order of the file.
    fn sorted_records(&self) -> Vec<&MemoryRecord> {
        let mut v: Vec<&MemoryRecord> = self
            .records
            .iter()
            .filter(|r| self.shown_record(r))
            .collect();
        v.sort_by_key(|r| KINDS.iter().position(|k| *k == r.record_type));
        v
    }

    fn projects(&self) -> Vec<String> {
        let mut v: Vec<String> = self
            .records
            .iter()
            .filter(|r| r.status != RecordStatus::Forgotten)
            .filter_map(|r| r.project.clone())
            .collect();
        v.sort();
        v.dedup();
        v
    }

    fn count_kind(&self, t: Option<RecordType>) -> usize {
        self.records
            .iter()
            .filter(|r| r.status == RecordStatus::Active && t.is_none_or(|t| r.record_type == t))
            .count()
    }
}

impl State {
    pub fn on_memory(&self) -> bool {
        self.memory_open && self.view == super::tasks::View::Meetings && self.selected.is_none()
    }

    pub fn open_memory(&mut self) {
        if !self.config.features.memory {
            return;
        }
        self.go_home();
        self.memory_open = true;
        self.memory.panel = false;
        self.memory.refresh(true);
        self.migration_prepare(false);
    }

    pub fn open_record(&mut self, id: &str) {
        self.memory.cursor = Some(id.to_string());
        self.memory.panel = true;
        self.memory.editing = false;
        self.memory.ask = None;
        self.memory.edit = TextEdit::default();
    }

    pub fn close_record(&mut self) {
        self.memory.panel = false;
        self.memory.editing = false;
        self.memory.ask = None;
        self.memory.edit = TextEdit::default();
    }

    fn memory_done(&mut self, key: &str, result: std::io::Result<()>) -> bool {
        match result {
            Ok(()) => {
                self.memory.errors.remove(key);
                self.memory.refresh(false);
                true
            }
            Err(e) => {
                self.memory
                    .errors
                    .insert(key.to_string(), format!("Not saved. {e}."));
                false
            }
        }
    }

    pub fn memory_confirm(&mut self, id: &str) {
        let r = MemoryStore::default().confirm_record(id);
        self.memory_done(id, r);
    }

    pub fn memory_retire(&mut self, id: &str) {
        let r = MemoryStore::default().retire_record(id);
        self.memory.ask = None;
        self.memory_done(id, r);
    }

    pub fn memory_forget(&mut self, id: &str) {
        let r = MemoryStore::default().forget_record(id);
        self.memory.ask = None;
        if self.memory_done(id, r) {
            self.close_record();
        }
    }

    pub fn memory_edit_start(&mut self, id: &str) {
        let Some(text) = self.memory.record(id).and_then(|r| r.text.clone()) else {
            return;
        };
        self.memory.edit = TextEdit::new(&text);
        self.memory.edit.focused = true;
        self.memory.editing = true;
        self.memory.ask = None;
    }

    pub fn memory_edit_save(&mut self) {
        let Some(id) = self.memory.cursor.clone() else {
            return;
        };
        let text = self.memory.edit.text.trim().to_string();
        if text.is_empty() {
            self.memory
                .errors
                .insert(id, "A claim needs some text.".into());
            return;
        }
        let r = MemoryStore::default().edit_record(&id, &text);
        if self.memory_done(&id, r) {
            self.memory.editing = false;
            self.memory.edit = TextEdit::default();
        }
    }

    pub fn proposal_accept(&mut self, id: &str) {
        let r = MemoryStore::default().accept_proposal(id);
        self.memory_done(id, r);
    }

    pub fn proposal_reject(&mut self, id: &str) {
        let r = MemoryStore::default().reject_proposal(id);
        self.memory_done(id, r);
    }

    pub fn migration_accept(&mut self) {
        if self.memory.preview_running {
            return;
        }
        let Some(fingerprint) = self
            .memory
            .preview
            .as_ref()
            .map(|p| p.source_fingerprint.clone())
        else {
            return;
        };
        let r = MemoryStore::default()
            .accept_migration(&fingerprint)
            .map(|_| ());
        if self.memory_done("migration", r) {
            self.memory.tab = Tab::All;
            self.close_record();
        }
    }

    pub fn migration_prepare(&mut self, force: bool) {
        if self.memory.migrated
            || self.memory.preview_running
            || (!force && self.memory.preview.as_ref().is_some_and(|p| p.prepared))
        {
            return;
        }
        self.memory.preview_running = true;
        self.memory.errors.remove("migration");
        let events = self.events_tx.clone();
        let waker = self.waker.clone();
        std::thread::spawn(move || {
            let result = MemoryStore::default()
                .prepare_migration(force)
                .map(|_| ())
                .map_err(|e| e.to_string());
            let _ = events.send(super::Event::MigrationPrepared(result));
            if let Some(waker) = waker {
                waker();
            }
        });
    }
}

// ---- rows ----------------------------------------------------------------------------------

#[derive(Clone, Copy)]
struct Lay {
    compact: bool,
    project: bool,
}

fn status_badge(s: RecordStatus, p: Palette) -> El {
    match s {
        RecordStatus::Active => badge("Active", p.ok, Some(p.okbg)),
        RecordStatus::Proposed => badge("Proposed", p.open, Some(p.chip)),
        RecordStatus::Superseded => badge("Replaced", p.faint, Some(p.chip)),
        RecordStatus::Retired => badge("Retired", p.faint, Some(p.chip)),
        RecordStatus::Forgotten => badge("Forgotten", p.faint, Some(p.chip)),
    }
}

fn row_shell(
    content: El,
    key: String,
    name: String,
    selected: bool,
    at_cursor: bool,
    id: String,
    p: Palette,
) -> El {
    let mut line = pressable(content)
        .key(key)
        .label(name)
        .w_full()
        .px(8.0)
        .radius(6.0)
        .hover(p.chip)
        .on_press(move |s: &mut State| s.open_record(&id));
    if selected {
        line = line
            .fill(p.sel)
            .child(el().w(2.0).h(18.0).radius(1.0).fill(p.accent).pin(
                Some(0.0),
                Some(10.0),
                None,
                None,
            ));
    } else if at_cursor {
        line = line.fill(p.chip).outline(1.0, 0.0, p.line);
    }
    line
}

fn flags(selected: bool, at_cursor: bool) -> (bool, bool) {
    (selected, at_cursor)
}

fn record_row(s: &State, r: &MemoryRecord, lay: Lay, p: Palette) -> El {
    let m = &s.memory;
    let selected = m.panel && m.cursor.as_deref() == Some(r.id.as_str());
    let at_cursor = m.keyboard && m.cursor.as_deref() == Some(r.id.as_str()) && !selected;
    let (selected, at_cursor) = flags(selected, at_cursor);
    let claim = plain(r.text.as_deref().unwrap_or_default());
    let review = needs_review(r, m.today);
    let project = r.project.clone().unwrap_or_default();
    let sources = plural(r.sources.len(), "source", "sources");
    let ink = if r.status == RecordStatus::Active {
        p.ink
    } else {
        p.faint
    };
    let content = if lay.compact {
        let mut meta = row().gap(8.0).items_center().min_w(0.0);
        if !project.is_empty() {
            meta = meta
                .child(roles::META.line(&project, p.faint))
                .child(roles::META.wrap("\u{b7}", p.faint));
        }
        meta = meta
            .child(roles::META.wrap(confirmed_text(r), if review { p.open } else { p.faint }))
            .child(roles::META.wrap("\u{b7}", p.faint))
            .child(roles::META.wrap(sources, p.faint));
        row()
            .w_full()
            .h(54.0)
            .gap(10.0)
            .items_center()
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(roles::SMALL.medium().line(&claim, ink))
                    .child(meta),
            )
            .child(status_badge(r.status, p))
    } else {
        let mut cells = row()
            .w_full()
            .h(38.0)
            .gap(10.0)
            .items_center()
            .child(roles::SMALL.medium().line(&claim, ink).grow(1.0));
        if lay.project {
            cells = cells.child(
                row()
                    .w(150.0)
                    .items_center()
                    .gap(6.0)
                    .child(if project.is_empty() {
                        el()
                    } else {
                        dot(7.0, p.soft)
                    })
                    .child(roles::META.line(&project, p.faint)),
            );
        }
        cells
            .child(
                row()
                    .w(84.0)
                    .items_center()
                    .child(status_badge(r.status, p)),
            )
            .child(row().w(116.0).items_center().child(if review {
                roles::META.medium().wrap(confirmed_text(r), p.open)
            } else {
                roles::meta(confirmed_text(r), p)
            }))
            .child(
                row()
                    .w(76.0)
                    .justify_end()
                    .items_center()
                    .child(roles::META.wrap(sources, p.faint)),
            )
    };
    row_shell(
        content,
        format!("mrow-{}", r.id),
        format!("Open the claim: {claim}"),
        selected,
        at_cursor,
        r.id.clone(),
        p,
    )
}

fn proposal_row(s: &State, pp: &PendingProposal, lay: Lay, p: Palette) -> El {
    let m = &s.memory;
    let pr = &pp.proposal;
    let selected = m.panel && m.cursor.as_deref() == Some(pr.id.as_str());
    let at_cursor = m.keyboard && m.cursor.as_deref() == Some(pr.id.as_str()) && !selected;
    let claim = plain(&pr.text);
    let project = pr.project.clone().unwrap_or_default();
    let sources = plural(pr.sources.len(), "source", "sources");
    let content = if lay.compact {
        row()
            .w_full()
            .h(54.0)
            .gap(10.0)
            .items_center()
            .child(
                col()
                    .grow(1.0)
                    .min_w(0.0)
                    .gap(2.0)
                    .child(roles::SMALL.medium().line(&claim, p.ink))
                    .child(roles::META.line(
                        format!(
                            "{} \u{b7} {sources}",
                            if project.is_empty() {
                                "No project"
                            } else {
                                &project
                            }
                        ),
                        p.faint,
                    )),
            )
            .child(badge(op_name(pr.op), p.accent, Some(p.chip)))
    } else {
        let mut cells = row()
            .w_full()
            .h(38.0)
            .gap(10.0)
            .items_center()
            .child(row().w(72.0).items_center().child(badge(
                op_name(pr.op),
                p.accent,
                Some(p.chip),
            )))
            .child(roles::SMALL.medium().line(&claim, p.ink).grow(1.0));
        if lay.project {
            cells = cells.child(
                row()
                    .w(150.0)
                    .items_center()
                    .gap(6.0)
                    .child(if project.is_empty() {
                        el()
                    } else {
                        dot(7.0, p.soft)
                    })
                    .child(roles::META.line(&project, p.faint)),
            );
        }
        cells
            .child(
                row()
                    .w(76.0)
                    .justify_end()
                    .items_center()
                    .child(roles::META.wrap(sources, p.faint)),
            )
            .child(
                row().w(64.0).justify_end().items_center().child(
                    roles::META.wrap(
                        parse_day(&pp.created_at)
                            .map_or(String::new(), |d| d.format("%-d %b").to_string()),
                        p.faint,
                    ),
                ),
            )
    };
    row_shell(
        content,
        format!("mprop-{}", pr.id),
        format!("Open the proposal: {claim}"),
        selected,
        at_cursor,
        pr.id.clone(),
        p,
    )
}

fn candidate_row(
    s: &State,
    c: &sidenote::core::memory::MigrationCandidate,
    lay: Lay,
    p: Palette,
) -> El {
    let m = &s.memory;
    let id = &c.record.id;
    let selected = m.panel && m.cursor.as_deref() == Some(id.as_str());
    let at_cursor = m.keyboard && m.cursor.as_deref() == Some(id.as_str()) && !selected;
    let claim = plain(c.record.text.as_deref().unwrap_or_default());
    let mut tags = row().gap(6.0).items_center();
    if c.ambiguous {
        tags = tags.child(badge("May hold more than one claim", p.open, Some(p.chip)));
    }
    if c.uncited {
        tags = tags.child(badge("No citation", p.warn, Some(p.warnbg)));
    }
    let line_no = roles::META.wrap(format!("Line {}", c.line), p.faint);
    let content = if lay.compact {
        row().w_full().h(54.0).gap(10.0).items_center().child(
            col()
                .grow(1.0)
                .min_w(0.0)
                .gap(2.0)
                .child(roles::SMALL.medium().line(&claim, p.ink))
                .child(row().gap(8.0).items_center().child(line_no).child(tags)),
        )
    } else {
        row()
            .w_full()
            .h(38.0)
            .gap(10.0)
            .items_center()
            .child(roles::SMALL.medium().line(&claim, p.ink).grow(1.0))
            .child(row().items_center().child(tags))
            .child(row().w(56.0).justify_end().child(line_no))
    };
    row_shell(
        content,
        format!("mcand-{id}"),
        format!("Open the candidate: {claim}"),
        selected,
        at_cursor,
        id.clone(),
        p,
    )
}

fn group_head(name: &str, n: usize, p: Palette) -> El {
    row()
        .w_full()
        .h(32.0)
        .px(8.0)
        .gap(8.0)
        .items_center()
        .child(roles::GROUP.wrap(name, p.ink))
        .child(roles::META.medium().wrap(n.to_string(), p.faint))
}

// ---- the top of the page -------------------------------------------------------------------

fn summary_line(s: &State, p: Palette) -> El {
    let m = &s.memory;
    let text_ = if !m.migrated {
        match &m.preview {
            Some(v) => format!(
                "Showing the old memory text. {} are ready to import when you approve.",
                plural(v.imported, "claim", "claims")
            ),
            None => "Showing the old memory text.".to_string(),
        }
    } else {
        let active = m.count_kind(None);
        let review = m
            .records
            .iter()
            .filter(|r| needs_review(r, m.today))
            .count();
        let mut parts = vec![format!("{active} active")];
        if review > 0 {
            parts.push(format!("{review} to review"));
        }
        if !m.proposals.is_empty() {
            parts.push(plural(
                m.proposals.len(),
                "proposal waiting",
                "proposals waiting",
            ));
        }
        format!("{}.", parts.join(", "))
    };
    roles::SMALL.wrap(text_, p.soft)
}

fn migration_banner(s: &State, narrow: bool, p: Palette) -> El {
    let m = &s.memory;
    let Some(v) = &m.preview else {
        return el();
    };
    let mut counts = row()
        .flex_wrap()
        .gaps(6.0, 6.0)
        .child(badge(
            format!("{} to import", v.imported),
            p.ink,
            Some(p.chip),
        ))
        .child(badge(
            format!("{} with no citation", v.uncited),
            if v.uncited > 0 { p.warn } else { p.soft },
            Some(if v.uncited > 0 { p.warnbg } else { p.chip }),
        ))
        .child(badge(
            format!("{} may hold more than one claim", v.ambiguous),
            if v.ambiguous > 0 { p.open } else { p.soft },
            Some(p.chip),
        ));
    for kind in KINDS {
        let n = v
            .candidates
            .iter()
            .filter(|c| c.record.record_type == kind)
            .count();
        counts = counts.child(badge(
            format!("{} {n}", kind_label(kind)),
            p.soft,
            Some(p.chip),
        ));
    }
    let skim = button(
        if m.before == Before::Candidates {
            "Show the old text"
        } else {
            "Skim the claims"
        },
        "mig-skim",
        false,
        p,
    )
    .label("Switch between the old text and the claims that would be imported")
    .on_press(|s: &mut State| {
        s.memory.before = if s.memory.before == Before::Candidates {
            Before::Text
        } else {
            Before::Candidates
        };
        s.close_record();
    });
    let accept = button(
        if m.preview_running {
            "Preparing claims..."
        } else {
            "Accept migration"
        },
        "mig-accept",
        true,
        p,
    )
    .label("Accept the migration")
    .on_press(|s: &mut State| s.migration_accept());
    let mut card = col()
        .key("mem-banner")
        .w_full()
        .gap(10.0)
        .px(if narrow { 12.0 } else { 16.0 })
        .py(14.0)
        .radius(12.0)
        .fill(p.warnbg)
        .child(roles::TITLE.wrap("Memory has not moved to the new format yet", p.ink))
        .child(roles::META.wrap("The new format keeps one claim per row, with its sources, so you can confirm, edit or retire each one. Nothing changes until you accept. Your current text stays exactly as it is in company.legacy.md.", p.soft))
        .child(counts)
        .child(
            row()
                .flex_wrap()
                .gaps(8.0, 8.0)
                .items_center()
                .child(accept)
                .child(skim)
                .child(button("Run preview again", "mig-rerun", false, p)
                    .on_press(|s: &mut State| s.migration_prepare(true))),
        );
    if m.preview_running {
        card = card.child(roles::META.wrap(
            "Splitting and typing the claims in the background. You can keep using Meetings.",
            p.soft,
        ));
    }
    for (title, issues) in [
        ("Unplaced text", &v.unplaced),
        ("Kept the original paragraph", &v.fallbacks),
    ] {
        for issue in issues {
            card = card.child(roles::META.wrap(
                format!(
                    "{title}, line {}: {}\n{}",
                    issue.line, issue.reason, issue.text
                ),
                p.warn,
            ));
        }
    }
    if let Some(e) = m.errors.get("migration") {
        card = card.child(roles::SMALL.medium().wrap(e, p.warn).role("alert"));
    }
    card
}

fn tab_pills(s: &State, p: Palette) -> El {
    let m = &s.memory;
    let mut bar = row().w_full().flex_wrap().items_center().gaps(6.0, 6.0);
    let n = m.proposals.len();
    bar = bar.child(
        pill(
            &format!("Review {n}"),
            "mtab-review",
            m.tab == Tab::Review,
            p,
        )
        .label("Proposals from the daily check")
        .on_press(|s: &mut State| set_tab(s, Tab::Review)),
    );
    bar = bar.child(
        pill(
            &format!("All {}", m.count_kind(None)),
            "mtab-all",
            m.tab == Tab::All,
            p,
        )
        .on_press(|s: &mut State| set_tab(s, Tab::All)),
    );
    for t in KINDS {
        bar = bar.child(
            pill(
                &format!("{} {}", kind_label(t), m.count_kind(Some(t))),
                &format!("mtab-{t:?}"),
                m.tab == Tab::Kind(t),
                p,
            )
            .on_press(move |s: &mut State| set_tab(s, Tab::Kind(t))),
        );
    }
    bar
}

fn set_tab(s: &mut State, t: Tab) {
    s.memory.tab = t;
    s.close_record();
    s.memory.cursor = None;
}

fn toolbar(s: &State, k: &Kit, p: Palette, layer: &mut Vec<El>) -> El {
    let m = &s.memory;
    let search = TextInput::new(&m.search)
        .plain()
        .placeholder("Search memory")
        .on_edit(|s: &mut State| &mut s.memory.search)
        .on_key(search_key)
        .height(36.0)
        .build(k)
        .key("mem-search")
        .border(1.0, if m.search.focused { p.accent } else { p.line })
        .radius(8.0)
        .fill(p.card)
        .grow(1.0)
        .min_w(72.0);
    let scope = col().key("mem-scope").child(
        SegmentedControl::new(
            &["Active", "Needs review", "Retired"],
            Some(match m.scope {
                Scope::Active => 0,
                Scope::NeedsReview => 1,
                Scope::Retired => 2,
            }),
        )
        .label("Show")
        .width(Some(330.0))
        .on_pick::<State>(|s, i, _| {
            s.memory.scope = [Scope::Active, Scope::NeedsReview, Scope::Retired][i];
            s.close_record();
        })
        .build(k),
    );
    let mut bar = row()
        .w_full()
        .flex_wrap()
        .items_center()
        .gaps(8.0, 8.0)
        .child(search)
        .child(scope);
    let names = m.projects();
    if names.len() > 1 {
        let shown = m.project.clone().unwrap_or_else(|| "All projects".into());
        let trigger = pressable(
            row()
                .gap(6.0)
                .items_center()
                .child(roles::META.wrap("Project", p.faint))
                .child(roles::GROUP.wrap(shown.clone(), p.ink))
                .child(label("\u{25be}", 10.0, 400.0, p.faint)),
        )
        .key("mem-project")
        .label(format!("Project filter: {shown}"))
        .px(10.0)
        .py(7.0)
        .radius(8.0)
        .border(1.0, p.line)
        .fill(p.card)
        .hover(p.chip)
        .on_press(|s: &mut State| s.memory.project_menu = !s.memory.project_menu);
        let mut menu = Menu::new("mem-project-menu")
            .trigger(trigger)
            .open(m.project_menu)
            .placement(Placement::BottomEnd)
            .min_width(220.0);
        if m.project_menu {
            menu = menu.on_close(|s: &mut State| s.memory.project_menu = false);
            menu = menu.item(
                MenuItem::new("All projects")
                    .checked(m.project.is_none())
                    .on_select(|s: &mut State| {
                        s.memory.project = None;
                        s.memory.project_menu = false;
                    }),
            );
            for n in names {
                let on = m.project.as_deref() == Some(n.as_str());
                menu = menu.item(MenuItem::new(n.clone()).checked(on).on_select(
                    move |s: &mut State| {
                        s.memory.project = Some(n.clone());
                        s.memory.project_menu = false;
                    },
                ));
            }
        }
        bar = bar.child(menu.build(k, layer));
    }
    bar
}

fn search_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        s.memory.search = TextEdit::default();
        tx.blur();
        return true;
    }
    false
}

fn empty_note(head: &str, body: &str, p: Palette) -> El {
    col()
        .w_full()
        .gap(6.0)
        .pt(36.0)
        .items_center()
        .child(roles::SECTION.wrap(head, p.ink))
        .child(roles::SMALL.wrap(body, p.soft))
}

fn record_list(s: &State, lay: Lay, p: Palette) -> El {
    let m = &s.memory;
    let recs = m.sorted_records();
    if recs.is_empty() {
        let (head, body) = if m.records.is_empty() {
            (
                "No memory yet",
                "Claims appear here after the daily check proposes them and you approve.",
            )
        } else if m.scope == Scope::NeedsReview {
            (
                "Nothing to review",
                "Every active claim was confirmed in the last 90 days.",
            )
        } else {
            (
                "Nothing matches",
                "Try another tab, another filter or a different search.",
            )
        };
        return empty_note(head, body, p);
    }
    let mut out = col().w_full();
    let grouped = m.tab == Tab::All;
    for kind in KINDS {
        let of: Vec<&&MemoryRecord> = recs.iter().filter(|r| r.record_type == kind).collect();
        if of.is_empty() {
            continue;
        }
        let mut g = col().w_full().mt(roles::GROUP_TOP).gap(1.0);
        if grouped {
            g = g.child(group_head(kind_label(kind), of.len(), p));
        }
        for r in of {
            g = g.child(record_row(s, r, lay, p));
        }
        out = out.child(g);
    }
    out
}

fn proposal_list(s: &State, lay: Lay, p: Palette) -> El {
    let m = &s.memory;
    if m.proposals.is_empty() {
        return empty_note(
            "No proposals waiting",
            "When the daily check suggests a change to memory, it waits here for you to accept or reject it.",
            p,
        );
    }
    let mut g = col().w_full().mt(10.0).gap(1.0);
    if !m.migrated {
        g = g.child(
            roles::SMALL.medium().wrap("Accept the migration first. Proposals cannot be accepted until memory is in the new format.", p.warn)
            .mb(6.0),
        );
    }
    for pp in &m.proposals {
        g = g.child(proposal_row(s, pp, lay, p));
    }
    g.child(
        roles::META
            .wrap(
                "Nothing changes until you accept. Rejecting leaves memory as it is.",
                p.faint,
            )
            .mt(10.0),
    )
}

fn candidate_list(s: &State, lay: Lay, p: Palette) -> El {
    let Some(v) = &s.memory.preview else {
        return empty_note("Nothing to import", "There is no legacy memory to move.", p);
    };
    let mut out = col().w_full();
    let mut groups: Vec<(String, Vec<&sidenote::core::memory::MigrationCandidate>)> = Vec::new();
    for c in &v.candidates {
        let name = c.record.project.clone().unwrap_or_else(|| "Company".into());
        match groups.iter_mut().find(|(n, _)| *n == name) {
            Some((_, v)) => v.push(c),
            None => groups.push((name, vec![c])),
        }
    }
    for (name, of) in groups {
        let mut g = col()
            .w_full()
            .mt(roles::GROUP_TOP)
            .gap(1.0)
            .child(group_head(&name, of.len(), p));
        for c in of {
            g = g.child(candidate_row(s, c, lay, p));
        }
        out = out.child(g);
    }
    out
}

fn list_column(
    s: &State,
    k: &Kit,
    p: Palette,
    narrow: bool,
    list_w: f32,
    layer: &mut Vec<El>,
) -> El {
    let m = &s.memory;
    let side = if narrow {
        16.0
    } else if m.panel {
        24.0
    } else {
        40.0
    };
    let lay = Lay {
        compact: narrow || list_w < 700.0,
        project: list_w >= 800.0,
    };
    let mut doc = col()
        .key("memory-top")
        .w_full()
        .max_w(920.0)
        .px(side)
        .pt(if narrow { 8.0 } else { 18.0 })
        .pb(56.0)
        .child(text_title("Memory", if narrow { 26.0 } else { 30.0 }, p).mb(4.0))
        .child(summary_line(s, p).mb(14.0));
    if !m.migrated {
        doc = doc.child(migration_banner(s, narrow, p).mb(14.0));
        match m.before {
            Before::Text => {
                doc = doc.child(legacy_text(s, k, p));
            }
            Before::Candidates => {
                doc = doc.child(candidate_list(s, lay, p));
            }
        }
    } else {
        doc = doc.child(tab_pills(s, p).mb(10.0));
        if m.tab != Tab::Review {
            doc = doc.child(toolbar(s, k, p, layer).mb(6.0));
        }
        doc = doc.child(if m.tab == Tab::Review {
            proposal_list(s, lay, p)
        } else {
            record_list(s, lay, p)
        });
        let forgotten = m
            .records
            .iter()
            .filter(|r| r.status == RecordStatus::Forgotten)
            .count();
        if forgotten > 0 && m.tab != Tab::Review {
            doc = doc.child(
                roles::META.wrap(format!(
                        "{} forgotten. Their text is gone from memory, and the same sources will not recreate them.",
                        plural(forgotten, "claim is", "claims are")
                    ), p.faint)
                .mt(16.0),
            );
        }
    }
    col().grow(1.0).min_w(0.0).min_h(0.0).child(scroll(
        col().w_full().items_center().child(doc),
        "memory-scroll",
    ))
}

fn legacy_text(s: &State, k: &Kit, p: Palette) -> El {
    let m = &s.memory;
    let mut c = col()
        .key("mem-legacy")
        .w_full()
        .gap(6.0)
        .child(spaced_caps("Current memory text", p));
    if m.legacy.trim().is_empty() {
        return c.child(roles::SMALL.wrap(
            "There is no company memory yet. New memory needs owner approval.",
            p.soft,
        ));
    }
    c = c.child(prose(s, k, p, &m.legacy, "mem-legacy", 15.0));
    c
}

// ---- the panel -----------------------------------------------------------------------------

fn source_block(s: &State, sources: &[MemorySource], key: &str, p: Palette) -> El {
    let mut c = col()
        .w_full()
        .gap(2.0)
        .child(spaced_caps("Sources", p).mb(2.0));
    if sources.is_empty() {
        c = c.child(roles::META.wrap("No source recorded.", p.faint));
    }
    for (n, src) in sources.iter().enumerate() {
        match src {
            MemorySource::Meeting { reference } => {
                c = c.child(source_row(s, reference, format!("{key}-src-{n}"), p));
            }
            MemorySource::Kb { path, revision } => {
                c = c.child(
                    col()
                        .w_full()
                        .gap(0.0)
                        .child(copy_row(s, path, format!("{key}-kb-{n}"), p))
                        .child(roles::META.wrap(
                            match revision {
                                Some(r) => {
                                    format!("KB revision {}", r.chars().take(7).collect::<String>())
                                }
                                None => "KB revision not recorded".to_string(),
                            },
                            p.faint,
                        )),
                );
            }
        }
    }
    c.child(
        roles::META
            .wrap(
                "A source shows where a claim came from. It does not prove the claim.",
                p.faint,
            )
            .mt(4.0),
    )
}

fn event_name(op: &str) -> String {
    match op {
        "accept_migration" => "Imported from the old memory text".into(),
        "accept_proposal" => "Accepted a proposal from the daily check".into(),
        "reject_proposal" => "Rejected a proposal".into(),
        "edit" => "Edited by you".into(),
        "confirm" => "Confirmed by you".into(),
        "retire" => "Retired by you".into(),
        "forget" => "Forgotten".into(),
        other => {
            let mut t = other.replace('_', " ");
            if let Some(f) = t.get(..1) {
                t = format!("{}{}", f.to_uppercase(), &t[1..]);
            }
            t
        }
    }
}

fn history_block(s: &State, r: &MemoryRecord, p: Palette) -> El {
    let m = &s.memory;
    let mut c = col()
        .w_full()
        .gap(3.0)
        .child(spaced_caps("History", p).mb(2.0));
    let mut lines: Vec<(String, String)> = m
        .events
        .iter()
        .filter(|e| {
            e.record_id == r.id || (e.record_id == "migration" && r.id.starts_with("legacy-"))
        })
        .map(|e| (day_text(&e.at), event_name(&e.operation)))
        .collect();
    if let Some(prev) = r.supersedes.as_ref().and_then(|id| m.record(id)) {
        lines.push((
            day_text(&prev.updated_at),
            format!(
                "Replaces: {}",
                plain(prev.text.as_deref().unwrap_or("an earlier claim"))
            ),
        ));
    }
    if let Some(next) = m
        .records
        .iter()
        .find(|x| x.supersedes.as_deref() == Some(r.id.as_str()))
    {
        lines.push((
            day_text(&next.updated_at),
            format!(
                "Replaced by: {}",
                plain(next.text.as_deref().unwrap_or_default())
            ),
        ));
    }
    if lines.is_empty() {
        c = c.child(roles::META.wrap("No changes recorded since it was added.", p.faint));
    }
    for (when, what) in lines.into_iter().rev() {
        c = c.child(
            row()
                .w_full()
                .gap(10.0)
                .items_start()
                .child(roles::META.medium().wrap(when, p.faint).w(78.0))
                .child(roles::META.wrap(what, p.soft).grow(1.0).min_w(0.0)),
        );
    }
    c
}

fn edit_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    if e.key == Key::Escape {
        s.memory.editing = false;
        s.memory.edit = TextEdit::default();
        tx.blur();
        return true;
    }
    false
}

fn record_panel(s: &State, k: &Kit, r: &MemoryRecord, overlay: bool, p: Palette) -> El {
    let m = &s.memory;
    let id = r.id.clone();
    let text_ = r.text.clone().unwrap_or_default();
    let review = needs_review(r, m.today);
    let badges = row()
        .flex_wrap()
        .gaps(6.0, 6.0)
        .items_center()
        .child(badge(kind_one(r.record_type), p.soft, Some(p.chip)))
        .child(status_badge(r.status, p));
    let mut body = col()
        .w_full()
        .gap(16.0)
        .px(20.0)
        .pt(4.0)
        .pb(24.0)
        .child(badges);
    if m.editing {
        body = body.child(
            TextArea::new(&m.edit)
                .label("Claim text")
                .auto_grow(5, 14)
                .on_edit(|s: &mut State| &mut s.memory.edit)
                .on_key(edit_key)
                .build(k)
                .key("mem-edit")
                .min_w(0.0),
        );
    } else {
        body = body.child(prose(s, k, p, &text_, &format!("mrow-{}", r.id), 15.0));
    }
    let mut props = col().w_full().gap(6.0);
    if let Some(pr) = &r.project {
        props = props.child(prop_row(
            "Project",
            row()
                .gap(8.0)
                .items_center()
                .min_h(24.0)
                .child(dot(7.0, p.soft))
                .child(roles::CONTROL.wrap(pr, p.ink)),
            p,
        ));
    }
    if !r.people.is_empty() {
        props = props.child(prop_row(
            "People",
            roles::SMALL.wrap(r.people.join(", "), p.ink).mt(2.0),
            p,
        ));
    }
    props = props
        .child(prop_row(
            "Basis",
            roles::SMALL.wrap(basis_name(r.basis), p.ink).mt(2.0),
            p,
        ))
        .child(prop_row(
            "First seen",
            roles::META
                .wrap(
                    r.first_seen
                        .as_deref()
                        .map_or_else(|| "Not recorded".to_string(), day_text),
                    if r.first_seen.is_some() {
                        p.ink
                    } else {
                        p.faint
                    },
                )
                .mt(2.0),
            p,
        ))
        .child(prop_row(
            "Confirmed",
            if review { roles::NAME } else { roles::SMALL }
                .wrap(
                    r.last_confirmed
                        .as_deref()
                        .map_or_else(|| "Not confirmed yet".to_string(), day_text),
                    if review { p.open } else { p.ink },
                )
                .mt(2.0),
            p,
        ));
    body = body.child(props);
    if review && r.status == RecordStatus::Active {
        body = body.child(roles::META.wrap("Suggested for review: it has not been confirmed in the last 90 days. Age does not make a claim wrong.", p.soft));
    }
    body = body
        .child(hairline(p))
        .child(source_block(s, &r.sources, &format!("mp-{}", r.id), p))
        .child(hairline(p))
        .child(history_block(s, r, p));
    if let Some(e) = m.errors.get(&r.id) {
        body = body.child(roles::SMALL.medium().wrap(e, p.warn).role("alert"));
    }

    let mut foot = col().w_full().px(16.0).py(12.0).gap(8.0);
    match m.ask {
        Some(Ask::Forget) => {
            let kk = id.clone();
            foot = foot
                .child(roles::SMALL.bold().wrap("Forget this claim?", p.ink))
                .child(roles::META.wrap("Its text is removed from memory, and the same sources will not bring it back. Old daily check reports, the legacy backup and your meetings may still contain it.", p.soft))
                .child(
                    row()
                        .gap(8.0)
                        .items_center()
                        .child(
                            pressable(roles::CONTROL.wrap("Forget it", p.card))
                                .key("mem-forget-go")
                                .px(14.0)
                                .py(6.0)
                                .radius(8.0)
                                .fill(p.warn)
                                .on_press(move |s: &mut State| s.memory_forget(&kk)),
                        )
                        .child(button("Cancel", "mem-ask-cancel", false, p).on_press(|s: &mut State| s.memory.ask = None)),
                );
        }
        Some(Ask::Retire) => {
            let kk = id.clone();
            foot = foot
                .child(roles::SMALL.bold().wrap("Retire this claim?", p.ink))
                .child(roles::META.wrap("It leaves the active list and the memory the daily check reads. Its text stays here as history, under Retired.", p.soft))
                .child(
                    row()
                        .gap(8.0)
                        .items_center()
                        .child(button("Retire", "mem-retire-go", true, p).on_press(move |s: &mut State| s.memory_retire(&kk)))
                        .child(button("Cancel", "mem-ask-cancel", false, p).on_press(|s: &mut State| s.memory.ask = None)),
                );
        }
        None if m.editing => {
            foot = foot.child(
                row()
                    .gap(8.0)
                    .items_center()
                    .child(
                        button("Save", "mem-edit-save", true, p)
                            .on_press(|s: &mut State| s.memory_edit_save()),
                    )
                    .child(button("Cancel", "mem-edit-cancel", false, p).on_press_tx(
                        |s: &mut State, tx| {
                            s.memory.editing = false;
                            s.memory.edit = TextEdit::default();
                            tx.blur();
                        },
                    ))
                    .child(
                        roles::META
                            .wrap(
                                "Saving counts as your confirmation of the new wording.",
                                p.faint,
                            )
                            .grow(1.0)
                            .min_w(0.0),
                    ),
            );
        }
        None => {
            let mut buttons = row().w_full().gap(8.0).items_center().flex_wrap();
            if r.status == RecordStatus::Active {
                let (a, b) = (id.clone(), id.clone());
                buttons = buttons
                    .child(
                        button("Confirm", "mem-confirm", true, p)
                            .label("Confirm that this is still true")
                            .on_press(move |s: &mut State| s.memory_confirm(&a)),
                    )
                    .child(
                        button("Edit", "mem-edit", false, p)
                            .on_press(move |s: &mut State| s.memory_edit_start(&b)),
                    )
                    .child(
                        button("Retire", "mem-retire", false, p)
                            .on_press(|s: &mut State| s.memory.ask = Some(Ask::Retire)),
                    );
            } else {
                buttons = buttons.child(roles::META.wrap(
                    format!("{}. Its text stays here as history.", status_name(r.status)),
                    p.soft,
                ));
            }
            buttons = buttons.child(el().grow(1.0)).child(
                pressable(roles::SMALL.medium().wrap("Forget", p.warn))
                    .key("mem-forget")
                    .label("Forget this claim")
                    .px(10.0)
                    .py(6.0)
                    .radius(8.0)
                    .hover(p.warnbg)
                    .on_press(|s: &mut State| s.memory.ask = Some(Ask::Forget)),
            );
            foot = foot.child(buttons);
        }
    }
    panel_frame(
        "memory-panel",
        panel_top("Claim", "Memory", overlay, |s| s.close_record(), p),
        body,
        foot,
        p,
    )
}

fn before_after_block(before: Option<(&str, String)>, after: (&str, String), p: Palette) -> El {
    let mut c = col().w_full().gap(8.0);
    if let Some((name, text_)) = before {
        c = c.child(
            col()
                .w_full()
                .gap(4.0)
                .px(12.0)
                .py(10.0)
                .radius(10.0)
                .fill(p.side)
                .child(spaced_caps(name, p))
                .child(roles::SMALL.wrap(text_, p.soft).selectable(true)),
        );
    }
    c.child(
        col()
            .w_full()
            .gap(4.0)
            .px(12.0)
            .py(10.0)
            .radius(10.0)
            .fill(p.okbg)
            .child(spaced_caps(after.0, p))
            .child(roles::CONTROL.wrap(after.1, p.ink).selectable(true)),
    )
}

fn proposal_panel(s: &State, pp: &PendingProposal, overlay: bool, p: Palette) -> El {
    let m = &s.memory;
    let pr = &pp.proposal;
    let target = pr.target_id.as_deref().and_then(|id| m.record(id));
    let target_text = target.and_then(|t| t.text.clone()).unwrap_or_default();
    let title = match pr.op {
        ProposalOp::Add => "Add a new claim",
        ProposalOp::Confirm => "Confirm an existing claim",
        ProposalOp::Supersede => "Replace a claim",
        ProposalOp::Retire => "Retire a claim",
    };
    let diff = match pr.op {
        ProposalOp::Add => before_after_block(None, ("New claim", pr.text.clone()), p),
        ProposalOp::Confirm => before_after_block(
            Some(("Claim now", target_text.clone())),
            (
                "After",
                format!(
                    "Same claim, confirmed {}, with the new source below.",
                    m.today.format("%-d %b %Y")
                ),
            ),
            p,
        ),
        ProposalOp::Supersede => before_after_block(
            Some(("Claim now", target_text.clone())),
            ("Replaced by", pr.text.clone()),
            p,
        ),
        ProposalOp::Retire => before_after_block(
            Some(("Claim now", target_text.clone())),
            ("After", "Retired. The text stays in history.".to_string()),
            p,
        ),
    };
    let mut body = col()
        .w_full()
        .gap(16.0)
        .px(20.0)
        .pt(4.0)
        .pb(24.0)
        .child(roles::SECTION.wrap(title, p.ink).w_full())
        .child(
            row()
                .flex_wrap()
                .gaps(6.0, 6.0)
                .child(badge(op_name(pr.op), p.accent, Some(p.chip)))
                .child(badge(kind_one(pr.record_type), p.soft, Some(p.chip)))
                .child(match &pr.project {
                    Some(n) => badge(n.clone(), p.soft, Some(p.chip)),
                    None => el(),
                }),
        )
        .child(diff)
        .child(
            col()
                .w_full()
                .gap(4.0)
                .child(spaced_caps("Why the check suggests it", p))
                .child(roles::SMALL.wrap(&pr.rationale, p.ink)),
        );
    let stale = target.is_some_and(|t| {
        pr.target_updated_at
            .as_deref()
            .is_some_and(|u| u != t.updated_at)
    });
    if pr.op != ProposalOp::Add && target.is_none() {
        body = body.child(roles::SMALL.medium().wrap(
            "The claim this refers to is gone or no longer active. Accepting will refuse.",
            p.warn,
        ));
    } else if stale {
        body = body.child(roles::SMALL.medium().wrap("This claim changed after the check suggested the update. Accepting will refuse. Reject it and let the next check look again.", p.warn));
    }
    body = body
        .child(hairline(p))
        .child(source_block(s, &pr.sources, &format!("pp-{}", pr.id), p));
    if let Some(e) = m.errors.get(&pr.id) {
        body = body.child(roles::SMALL.medium().wrap(e, p.warn).role("alert"));
    }
    let id = pr.id.clone();
    let id2 = pr.id.clone();
    let mut foot = col().w_full().px(16.0).py(12.0).gap(8.0);
    if !m.migrated {
        foot = foot.child(
            roles::META
                .medium()
                .wrap("Accept the migration first, then accept this.", p.warn),
        );
    }
    let accept = if m.migrated {
        button("Accept", "prop-accept", true, p)
            .on_press(move |s: &mut State| s.proposal_accept(&id))
    } else {
        button("Accept", "prop-accept", false, p).dim(0.5)
    };
    foot = foot.child(
        row()
            .gap(8.0)
            .items_center()
            .child(accept.label("Accept this change to memory"))
            .child(
                button("Reject", "prop-reject", false, p)
                    .label("Reject this change")
                    .on_press(move |s: &mut State| s.proposal_reject(&id2)),
            )
            .child(
                roles::META
                    .wrap("Nothing changes until you accept.", p.faint)
                    .grow(1.0)
                    .min_w(0.0),
            ),
    );
    panel_frame(
        "memory-panel",
        panel_top("Proposal", "Memory", overlay, |s| s.close_record(), p),
        body,
        foot,
        p,
    )
}

fn candidate_panel(
    s: &State,
    k: &Kit,
    c: &sidenote::core::memory::MigrationCandidate,
    overlay: bool,
    p: Palette,
) -> El {
    let r = &c.record;
    let mut body = col()
        .w_full()
        .gap(16.0)
        .px(20.0)
        .pt(4.0)
        .pb(24.0)
        .child(
            row()
                .flex_wrap()
                .gaps(6.0, 6.0)
                .child(badge("Candidate", p.accent, Some(p.chip)))
                .child(badge(kind_one(r.record_type), p.soft, Some(p.chip)))
                .child(if c.ambiguous {
                    badge("May hold more than one claim", p.open, Some(p.chip))
                } else {
                    el()
                })
                .child(if c.uncited {
                    badge("No citation", p.warn, Some(p.warnbg))
                } else {
                    el()
                }),
        )
        .child(prose(
            s,
            k,
            p,
            r.text.as_deref().unwrap_or_default(),
            &format!("mcand-{}", r.id),
            15.0,
        ));
    let mut props = col().w_full().gap(6.0);
    if let Some(n) = &r.project {
        props = props.child(prop_row(
            "Group",
            row()
                .gap(8.0)
                .items_center()
                .min_h(24.0)
                .child(dot(7.0, p.soft))
                .child(roles::CONTROL.wrap(n, p.ink)),
            p,
        ));
    }
    props = props.child(prop_row(
        "Old text",
        roles::SMALL
            .wrap(format!("Line {} of company.md", c.line), p.ink)
            .mt(2.0),
        p,
    ));
    body = body
        .child(props)
        .child(hairline(p))
        .child(source_block(s, &r.sources, &format!("mc-{}", r.id), p))
        .child(roles::META.wrap("Imported dates stay unknown unless the old text states them. Approving the migration does not say each claim is freshly confirmed.", p.faint));
    let foot = row().w_full().px(16.0).py(12.0).child(roles::META.wrap(
        "This is a candidate. Nothing is saved until you accept the migration.",
        p.soft,
    ));
    panel_frame(
        "memory-panel",
        panel_top("Candidate", "Memory", overlay, |s| s.close_record(), p),
        body,
        foot,
        p,
    )
}

fn panel(s: &State, k: &Kit, overlay: bool, p: Palette) -> Option<El> {
    let m = &s.memory;
    if !m.panel {
        return None;
    }
    let id = m.cursor.as_deref()?;
    if !m.migrated {
        let c = m
            .preview
            .as_ref()?
            .candidates
            .iter()
            .find(|c| c.record.id == id)?;
        return Some(candidate_panel(s, k, c, overlay, p));
    }
    if m.tab == Tab::Review {
        let pp = m.proposals.iter().find(|x| x.proposal.id == id)?;
        return Some(proposal_panel(s, pp, overlay, p));
    }
    let r = m.record(id)?;
    Some(record_panel(s, k, r, overlay, p))
}

pub fn pane(s: &State, k: &Kit, p: Palette, narrow: bool, layer: &mut Vec<El>) -> El {
    let m = &s.memory;
    let probe = split_geometry(s, narrow, true);
    let shown = panel(s, k, probe.2, p);
    let (list_w, panel_w, overlay) = split_geometry(s, narrow, shown.is_some());
    let mut pane = col()
        .key("memory-pane")
        .grow(1.0)
        .min_w(0.0)
        .min_h(0.0)
        .w_full()
        .fill(p.bg);
    if let Some(note) = m.notice.as_ref().or(s.notice.as_ref()) {
        pane = pane.child(notice_bar(note, p));
    }
    pane.child(list_and_panel(
        list_column(s, k, p, narrow, list_w, layer),
        shown,
        panel_w,
        overlay,
        p,
    ))
}

// ---- the keyboard --------------------------------------------------------------------------

fn move_cursor(s: &mut State, delta: isize, tx: &mut Tx) {
    let ids = s.memory.shown_ids();
    if ids.is_empty() {
        return;
    }
    let at = s
        .memory
        .cursor
        .as_ref()
        .and_then(|c| ids.iter().position(|i| i == c));
    let next = match at {
        Some(i) if s.memory.keyboard => {
            (i as isize + delta).clamp(0, ids.len() as isize - 1) as usize
        }
        Some(i) => i,
        None if delta < 0 => ids.len() - 1,
        None => 0,
    };
    s.memory.keyboard = true;
    let id = ids[next].clone();
    let prefix = if !s.memory.migrated {
        "mcand"
    } else if s.memory.tab == Tab::Review {
        "mprop"
    } else {
        "mrow"
    };
    tx.scroll_into_view(format!("{prefix}-{id}"));
    if s.memory.panel {
        s.open_record(&id);
    } else {
        s.memory.cursor = Some(id);
    }
}

/// The Memory page's keys. Letters only count while no text box has the keyboard.
pub fn on_key(s: &mut State, e: &KeyEvent, tx: &mut Tx) -> bool {
    match &e.key {
        Key::Down => move_cursor(s, 1, tx),
        Key::Up => move_cursor(s, -1, tx),
        Key::Enter => match s.memory.cursor.clone() {
            Some(id) if s.memory.keyboard => s.open_record(&id),
            _ => return false,
        },
        Key::Escape => {
            if s.memory.ask.is_some() {
                s.memory.ask = None;
            } else if s.memory.panel {
                s.close_record();
            } else if s.memory.project_menu {
                s.memory.project_menu = false;
            } else if !s.memory.search.text.is_empty() {
                s.memory.search = TextEdit::default();
            } else {
                s.go_home();
            }
        }
        Key::Text(t) if t == "j" => move_cursor(s, 1, tx),
        Key::Text(t) if t == "k" => move_cursor(s, -1, tx),
        _ => return false,
    }
    true
}

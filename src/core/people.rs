//! Display-time roster from meeting names, owner identity and explicit team evidence.
use super::{Config, Meeting, TaskItem, Teams, UiPrefs};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    OurTeam,
    External,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct RosterRow {
    names: BTreeSet<String>,
    automatic: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Affiliations {
    pub your_name: String,
    pub company: Option<String>,
    members: Vec<RosterRow>,
    pub notice: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RosterKey {
    repo: std::path::PathBuf,
    commit: String,
    path: String,
}
#[derive(Default)]
struct RosterCache {
    reads: usize,
    entry: Option<(RosterKey, Option<Vec<RosterRow>>)>,
}
impl RosterCache {
    fn read(
        &mut self,
        key: RosterKey,
        read: impl FnOnce() -> Option<Vec<RosterRow>>,
    ) -> Option<Vec<RosterRow>> {
        if let Some((cached, rows)) = &self.entry
            && *cached == key
        {
            return rows.clone();
        }
        self.reads += 1;
        let rows = read();
        self.entry = Some((key, rows.clone()));
        rows
    }
}
static ROSTER_CACHE: std::sync::Mutex<RosterCache> = std::sync::Mutex::new(RosterCache {
    reads: 0,
    entry: None,
});

impl RosterKey {
    fn resolve(kb: &super::config::KnowledgeBase) -> std::io::Result<Self> {
        use std::{fs, path::PathBuf};
        kb.validate().map_err(super::store_io::invalid)?;
        let repo = PathBuf::from(&kb.repo_path);
        let dot_git = repo.join(".git");
        let git = if dot_git.is_file() {
            let text = fs::read_to_string(&dot_git)?;
            repo.join(
                text.trim()
                    .strip_prefix("gitdir: ")
                    .ok_or_else(|| super::store_io::invalid("Invalid Git directory"))?,
            )
        } else {
            dot_git
        };
        let common = match fs::read_to_string(git.join("commondir")) {
            Ok(path) => git.join(path.trim()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => git.clone(),
            Err(error) => return Err(error),
        };
        let mut reference = format!("refs/remotes/{}", kb.revision);
        for _ in 0..8 {
            let loose = fs::read_to_string(git.join(&reference))
                .or_else(|_| fs::read_to_string(common.join(&reference)));
            let commit = match loose {
                Ok(value) => {
                    if let Some(target) = value.trim().strip_prefix("ref: ") {
                        if !target.starts_with("refs/") || target.split('/').any(|p| p == "..") {
                            break;
                        }
                        reference = target.to_owned();
                        continue;
                    }
                    value.trim().to_owned()
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    fs::read_to_string(common.join("packed-refs"))?
                        .lines()
                        .filter_map(|line| line.split_once(' '))
                        .find_map(|(sha, name)| (name == reference).then(|| sha.to_owned()))
                        .ok_or_else(|| super::store_io::invalid("Missing roster revision"))?
                }
                Err(error) => return Err(error),
            };
            if matches!(commit.len(), 40 | 64) && commit.bytes().all(|c| c.is_ascii_hexdigit()) {
                return Ok(Self {
                    repo,
                    commit,
                    path: kb.people_roster_source.clone(),
                });
            }
            break;
        }
        Err(super::store_io::invalid("Invalid roster revision"))
    }
}

impl Affiliations {
    /// Diagnostic counter used to measure cold versus warm background roster reads.
    pub fn cache_reads() -> usize {
        ROSTER_CACHE.lock().unwrap_or_else(|e| e.into_inner()).reads
    }

    /// Startup identity is config-only: no roster filesystem access or subprocesses.
    pub fn identity(config: &Config) -> Self {
        Self {
            your_name: config
                .notes
                .as_ref()
                .map(|n| n.your_name.clone())
                .unwrap_or_default(),
            company: (!config.knowledge_base.company_name.trim().is_empty())
                .then(|| config.knowledge_base.company_name.trim().to_owned()),
            ..Default::default()
        }
    }

    /// Background-only refresh. A warm call reads local refs, never starts Git.
    pub fn load(config: &Config) -> Self {
        let mut result = Self::identity(config);
        let kb = &config.knowledge_base;
        if !kb.people_roster_source.ends_with(".md") {
            return result;
        }
        let rows = RosterKey::resolve(kb).ok().and_then(|cache_key| {
            let mut cache = ROSTER_CACHE.lock().unwrap_or_else(|e| e.into_inner());
            cache.read(cache_key.clone(), || {
                let bytes = super::daily_check::git(
                    &kb.repo_path,
                    &["show", &format!("{}:{}", cache_key.commit, cache_key.path)],
                )
                .ok()?;
                Self::parse_members(std::str::from_utf8(&bytes).ok()?)
            })
        });
        match rows {
            Some(rows) => result.members = rows,
            None => result.notice = Some("Could not read the configured Markdown team roster. Check the repository, revision and Person / Role table.".into()),
        }
        result
    }

    pub fn with_roster(config: &Config, text: &str) -> Self {
        Self {
            members: Self::parse_members(text).unwrap_or_default(),
            ..Self::identity(config)
        }
    }

    fn parse_members(text: &str) -> Option<Vec<RosterRow>> {
        let mut rows = Vec::new();
        let mut columns = None;
        for line in text.lines() {
            let line = line.trim();
            if !line.starts_with('|') || !line.ends_with('|') {
                if columns.is_some() {
                    break;
                }
                continue;
            }
            let cells: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
            if columns.is_none() {
                // Match the roster schema, never a generic contacts Name table.
                let person = cells.iter().position(|cell| key(cell) == "person");
                let role = cells.iter().any(|cell| key(cell).starts_with("role"));
                if let Some(person) = person.filter(|_| role) {
                    columns = Some((person, cells.iter().position(|cell| key(cell) == "aliases")));
                }
                continue;
            }
            let (person, aliases) = columns?;
            let Some(cell) = cells.get(person) else {
                continue;
            };
            if cell.chars().all(|c| matches!(c, '-' | ':' | ' ')) {
                continue;
            }
            let mut row = RosterRow::default();
            for (index, cell) in std::iter::once((person, *cell))
                .chain(aliases.and_then(|i| cells.get(i).map(|v| (i, *v))))
            {
                let clean = strip_brackets(cell);
                for value in clean.split(['/', ',']) {
                    let name = key(value.trim().trim_matches(['*', '`']));
                    if named(&name) {
                        if index != person || name.split_whitespace().count() > 1 {
                            row.automatic.insert(name.clone());
                        }
                        row.names.insert(name);
                    }
                }
            }
            if row
                .names
                .iter()
                .any(|name| name.split_whitespace().count() > 1)
            {
                row.automatic.extend(row.names.iter().cloned());
            }
            if !row.names.is_empty() {
                rows.push(row);
            }
        }
        (!rows.is_empty()).then_some(rows)
    }

    pub fn is_self(&self, name: &str, teams: &Teams) -> bool {
        let name = key(&teams.owner_name(name));
        let owner = key(&teams.owner_name(&self.your_name));
        if matches!(name.as_str(), "you" | "me") || (!owner.is_empty() && name == owner) {
            return true;
        }
        let Some(first) = owner.split_whitespace().next() else {
            return false;
        };
        if name != first {
            return false;
        }
        let matches: Vec<_> = self
            .members
            .iter()
            .filter(|row| row.names.contains(&name))
            .collect();
        matches.len() == 1
            && !self
                .members
                .iter()
                .filter(|row| *row != matches[0])
                .any(|row| {
                    row.names
                        .iter()
                        .any(|other| other.split_whitespace().next() == Some(first))
                })
    }

    pub fn manual_side(&self, name: &str, teams: &Teams, prefs: &UiPrefs) -> Option<Side> {
        prefs.people_sides.get(&key(name)).copied().or_else(|| {
            prefs
                .people_sides
                .get(&key(&teams.owner_name(name)))
                .copied()
        })
    }

    pub fn suggested(&self, name: &str, teams: &Teams, prefs: &UiPrefs) -> bool {
        self.side(name, teams, prefs).is_none()
            && self.members.iter().any(|row| {
                row.names.contains(&key(name)) || row.names.contains(&key(&teams.owner_name(name)))
            })
    }

    pub fn company_for(&self, name: &str, teams: &Teams) -> Option<&str> {
        (self.is_self(name, teams) || self.automatic(name, teams))
            .then_some(self.company.as_deref())
            .flatten()
    }

    fn automatic(&self, name: &str, teams: &Teams) -> bool {
        self.members.iter().any(|row| {
            row.automatic.contains(&key(name))
                || row.automatic.contains(&key(&teams.owner_name(name)))
        })
    }

    pub fn side(&self, name: &str, teams: &Teams, prefs: &UiPrefs) -> Option<Side> {
        self.manual_side(name, teams, prefs).or_else(|| {
            (self.is_self(name, teams)
                || self.automatic(name, teams)
                || teams.resolve(Some(name), None) != "No team")
                .then_some(Side::OurTeam)
        })
    }
}

fn strip_brackets(text: &str) -> String {
    let mut depth = 0usize;
    text.chars()
        .filter(|c| match c {
            '(' | '[' | '{' => {
                depth += 1;
                false
            }
            ')' | ']' | '}' => {
                depth = depth.saturating_sub(1);
                false
            }
            _ => depth == 0,
        })
        .collect()
}

pub fn key(name: &str) -> String {
    name.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
fn meeting_key(m: &Meeting) -> String {
    format!("{}/{}", m.device.as_deref().unwrap_or("local"), m.id)
}
fn named(name: &str) -> bool {
    let k = key(name);
    !k.is_empty()
        && !matches!(k.as_str(), "you" | "me" | "them" | "unknown")
        && !k.starts_with("speaker ")
        && !k.starts_with("them-")
}
fn names(m: &Meeting, teams: &Teams) -> BTreeSet<String> {
    m.summary
        .people
        .iter()
        .map(String::as_str)
        .filter(|n| named(n))
        .map(|n| teams.owner_name(n))
        .collect()
}

/// Freeze the already-present corpus once. Only new ids created after this rollout
/// make people eligible; opening People never reclassifies an old meeting.
pub fn initialize(prefs: &mut UiPrefs, meetings: &[Meeting]) -> bool {
    if prefs.people_existing_meetings.is_some() && prefs.people_since_ms.is_some() {
        return false;
    }
    prefs
        .people_existing_meetings
        .get_or_insert_with(|| meetings.iter().map(meeting_key).collect());
    prefs.people_since_ms = Some(chrono::Local::now().timestamp_millis());
    true
}

pub struct Person {
    pub name: String,
    pub aliases: Vec<String>,
    pub team: String,
    pub meetings: usize,
    pub last_seen: Option<NaiveDate>,
    pub open: usize,
    pub side: Option<Side>,
    pub company: Option<String>,
    pub is_self: bool,
    pub suggested: bool,
    pub manual_side: bool,
}

pub fn roster(
    meetings: &[Meeting],
    teams: &Teams,
    tasks: &[TaskItem],
    prefs: &UiPrefs,
    affiliations: &Affiliations,
) -> Vec<Person> {
    let Some(existing) = &prefs.people_existing_meetings else {
        return Vec::new();
    };
    let Some(since) = prefs.people_since_ms else {
        return Vec::new();
    };
    let eligible: BTreeSet<_> = meetings
        .iter()
        .filter(|m| {
            !existing.contains(&meeting_key(m))
                && m.started.is_some_and(|d| d.timestamp_millis() >= since)
        })
        .flat_map(|m| names(m, teams))
        .map(|n| key(&n))
        .collect();
    if eligible.is_empty() {
        return Vec::new();
    }
    let mut people = BTreeMap::<String, Person>::new();
    for m in meetings {
        // Multiple aliases of one person still count once per meeting.
        let mut seen = BTreeSet::new();
        for name in names(m, teams) {
            let id = key(&name);
            if !eligible.contains(&id) || !seen.insert(id.clone()) {
                continue;
            }
            let team = teams.resolve(Some(&name), None);
            let side = affiliations.side(&name, teams, prefs);
            let person = people.entry(id).or_insert_with(|| Person {
                aliases: teams
                    .teams
                    .iter()
                    .flat_map(|t| t.aliases.iter())
                    .filter(|(_, target)| key(target) == key(&name))
                    .map(|(alias, _)| alias.clone())
                    .collect(),
                company: (side == Some(Side::OurTeam))
                    .then_some(affiliations.company.as_deref())
                    .flatten()
                    .map(str::to_owned),
                suggested: affiliations.suggested(&name, teams, prefs),
                manual_side: affiliations.manual_side(&name, teams, prefs).is_some(),
                is_self: affiliations.is_self(&name, teams),
                name,
                team,
                side,
                meetings: 0,
                last_seen: None,
                open: 0,
            });
            person.meetings += 1;
            person.last_seen = person.last_seen.max(m.started.map(|d| d.date_naive()));
        }
    }
    for t in tasks.iter().filter(|t| !t.done) {
        if let Some(owner) = &t.owner {
            let id = key(&teams.owner_name(owner));
            if let Some(person) = people.get_mut(&id) {
                person.open += 1;
            }
        }
    }
    people.into_values().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn meetings() -> Vec<Meeting> {
        let root = tempfile::tempdir().unwrap();
        let folder = root.path().join("100-1");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("english-summary.json"),
            r#"{"title":"Old meeting","people":["Old Only","Alex C","You"]}"#,
        )
        .unwrap();
        std::fs::write(folder.join("manifest.json"), r#"{"created_at_ms":1000}"#).unwrap();
        let (meetings, _) = super::super::scan_in(root.path());
        meetings
    }
    fn affiliations() -> Affiliations {
        Affiliations {
            your_name: "Casey Morgan".into(),
            company: Some("Example Company".into()),
            members: Affiliations::parse_members(
                "# Team\n\n| Person | Role |\n|---|---|\n| Avery Reed / Avery R / A. Reed | Engineer |\n| Blake Bell, Drew Day | Designer |\n\nOutside contact: Jordan Guest",
            ).unwrap(),
            notice: None,
        }
    }

    #[test]
    fn identity_does_not_load_roster_or_resolve_repository() {
        let mut config = Config::default();
        config.knowledge_base.repo_path = "//unreachable/synthetic".into();
        config.knowledge_base.people_roster_source = "roster.md".into();
        let value = Affiliations::identity(&config);
        assert!(value.members.is_empty());
        assert!(value.notice.is_none());
    }

    #[test]
    fn cache_key_tracks_repo_resolved_commit_and_roster_path() {
        let mut cache = RosterCache::default();
        let base = RosterKey {
            repo: "synthetic-a".into(),
            commit: "a".repeat(40),
            path: "people.md".into(),
        };
        cache.read(base.clone(), || Some(Vec::new()));
        cache.read(base.clone(), || panic!("warm refresh must not read roster"));
        assert_eq!(cache.reads, 1);
        for changed in [
            RosterKey {
                repo: "synthetic-b".into(),
                ..base.clone()
            },
            RosterKey {
                commit: "b".repeat(40),
                ..base.clone()
            },
            RosterKey {
                path: "other.md".into(),
                ..base
            },
        ] {
            cache.read(changed, || Some(Vec::new()));
        }
        assert_eq!(cache.reads, 4);
    }

    #[test]
    fn local_revision_resolution_handles_loose_packed_and_worktree_refs() {
        let root = tempfile::tempdir().unwrap();
        let repo = root.path().join("repo");
        let git = repo.join(".git");
        std::fs::create_dir_all(git.join("refs/remotes/origin")).unwrap();
        let mut kb = super::super::config::KnowledgeBase::default();
        kb.repo_path = repo.to_str().unwrap().into();
        kb.revision = "origin/main".into();
        kb.source_roots = vec!["docs".into()];
        kb.people_roster_source = "people.md".into();
        let loose = git.join("refs/remotes/origin/main");
        std::fs::write(&loose, "a".repeat(40)).unwrap();
        assert_eq!(RosterKey::resolve(&kb).unwrap().commit, "a".repeat(40));
        std::fs::remove_file(loose).unwrap();
        std::fs::write(
            git.join("packed-refs"),
            format!("{} refs/remotes/origin/main\n", "b".repeat(40)),
        )
        .unwrap();
        assert_eq!(RosterKey::resolve(&kb).unwrap().commit, "b".repeat(40));
        let worktree = root.path().join("lane");
        std::fs::create_dir_all(&worktree).unwrap();
        let lane = git.join("worktrees/lane");
        std::fs::create_dir_all(&lane).unwrap();
        std::fs::write(lane.join("commondir"), "../..").unwrap();
        std::fs::write(worktree.join(".git"), format!("gitdir: {}", lane.display())).unwrap();
        kb.repo_path = worktree.to_str().unwrap().into();
        assert_eq!(RosterKey::resolve(&kb).unwrap().commit, "b".repeat(40));
    }

    #[test]
    fn yaml_is_unsupported_without_a_permanent_notice() {
        let mut config = Config::default();
        config.knowledge_base.people_roster_source = "people.yaml".into();
        let value = Affiliations::load(&config);
        assert!(value.notice.is_none());
        assert!(value.members.is_empty());
    }

    #[test]
    fn first_name_only_is_a_suggestion_until_manually_confirmed() {
        let evidence = Affiliations {
            members: Affiliations::parse_members(
                "| Person | Role |\n|---|---|\n| Avery | Engineer |",
            )
            .unwrap(),
            ..Default::default()
        };
        let teams = Teams::default();
        let mut prefs = UiPrefs::default();
        assert_eq!(evidence.side("Avery", &teams, &prefs), None);
        assert!(evidence.suggested("Avery", &teams, &prefs));
        prefs.people_sides.insert(key("Avery"), Side::OurTeam);
        assert_eq!(evidence.side("Avery", &teams, &prefs), Some(Side::OurTeam));
        assert!(!evidence.suggested("Avery", &teams, &prefs));
    }

    #[test]
    fn roster_schema_and_brackets_exclude_contacts_and_annotations() {
        let rows = Affiliations::parse_members("| Name | Company |\n|---|---|\n| Guest Person | Example |\n\n| Person | Role | Aliases |\n|---|---|---|\n| Avery Reed (External Adviser) [Firm Name] | Engineer | AR |\n\n| Person | Role |\n|---|---|\n| Later Person | Adviser |").unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            rows[0].names,
            BTreeSet::from([key("Avery Reed"), key("AR")])
        );
        assert!(!rows[0].names.contains(&key("Firm Name")));
    }

    #[test]
    fn manual_external_hides_inferred_company() {
        let mut input = meetings();
        input[0].summary.people = vec!["Avery Reed".into()];
        let mut prefs = UiPrefs {
            people_existing_meetings: Some(BTreeSet::new()),
            people_since_ms: Some(0),
            ..Default::default()
        };
        prefs.people_sides.insert(key("Avery Reed"), Side::External);
        let people = roster(&input, &Teams::default(), &[], &prefs, &affiliations());
        assert_eq!(people[0].company, None);
        assert!(people[0].manual_side);
    }

    #[test]
    fn owner_identity_and_microphone_labels_are_self() {
        let evidence = affiliations();
        let teams = Teams::default();
        for name in [" CASEY  Morgan ", "You", "me"] {
            assert!(evidence.is_self(name, &teams));
            assert_eq!(
                evidence.side(name, &teams, &UiPrefs::default()),
                Some(Side::OurTeam)
            );
            assert_eq!(evidence.company_for(name, &teams), Some("Example Company"));
        }
        assert!(!Affiliations::default().is_self("Casey Morgan", &teams));
        assert!(!evidence.is_self("Them", &teams));
    }

    #[test]
    fn short_owner_name_requires_explicit_unambiguous_roster_evidence() {
        let mut evidence = affiliations();
        assert!(!evidence.is_self("Casey", &Teams::default()));
        evidence.members.push(RosterRow {
            names: BTreeSet::from([key("Casey"), key("Casey M")]),
            ..Default::default()
        });
        assert!(evidence.is_self("CASEY", &Teams::default()));
        assert!(!evidence.is_self("Case", &Teams::default()));
        evidence.members.push(RosterRow {
            names: BTreeSet::from([key("Casey Bell")]),
            ..Default::default()
        });
        assert!(!evidence.is_self("Casey", &Teams::default()));
    }

    #[test]
    fn roster_matches_only_exact_normalized_names_and_explicit_aliases() {
        let evidence = affiliations();
        for name in [
            "AVERY  REED",
            "avery r",
            "A. Reed",
            "Blake Bell",
            "Drew Day",
        ] {
            assert_eq!(
                evidence.side(name, &Teams::default(), &UiPrefs::default()),
                Some(Side::OurTeam)
            );
            assert_eq!(
                evidence.company_for(name, &Teams::default()),
                Some("Example Company")
            );
        }
    }

    #[test]
    fn manual_side_wins_over_roster_and_self_without_mutating_preferences() {
        let evidence = affiliations();
        let mut prefs = UiPrefs::default();
        for name in ["Avery Reed", "Casey Morgan"] {
            prefs.people_sides.insert(key(name), Side::External);
        }
        let before = prefs.clone();
        for _ in 0..2 {
            for name in ["Avery Reed", "Casey Morgan"] {
                assert_eq!(
                    evidence.side(name, &Teams::default(), &prefs),
                    Some(Side::External)
                );
            }
        }
        assert_eq!(prefs, before);
    }

    #[test]
    fn unknown_people_roles_and_partial_names_stay_unassigned() {
        let evidence = affiliations();
        for name in [
            "Avery",
            "Avery Reed Guest",
            "Jordan Guest",
            "Engineer",
            "someone@example.test",
            "",
        ] {
            assert_eq!(
                evidence.side(name, &Teams::default(), &UiPrefs::default()),
                None
            );
            assert_eq!(evidence.company_for(name, &Teams::default()), None);
        }
        assert!(Affiliations::parse_members("Avery Reed is on the team").is_none());
    }

    #[test]
    fn existing_people_get_projected_affiliation_idempotently() {
        let mut meetings = meetings();
        meetings[0].summary.people = vec![
            "Casey Morgan".into(),
            "Avery R".into(),
            "Jordan Guest".into(),
        ];
        let prefs = UiPrefs {
            people_existing_meetings: Some(BTreeSet::new()),
            people_since_ms: Some(0),
            ..Default::default()
        };
        let evidence = affiliations();
        for _ in 0..2 {
            let people = roster(&meetings, &Teams::default(), &[], &prefs, &evidence);
            assert_eq!(people.len(), 3);
            let owner = people.iter().find(|p| p.name == "Casey Morgan").unwrap();
            assert!(owner.is_self);
            assert_eq!(owner.side, Some(Side::OurTeam));
            let teammate = people.iter().find(|p| p.name == "Avery R").unwrap();
            assert_eq!(teammate.side, Some(Side::OurTeam));
            assert_eq!(teammate.company.as_deref(), Some("Example Company"));
            assert_eq!(
                people
                    .iter()
                    .find(|p| p.name == "Jordan Guest")
                    .unwrap()
                    .side,
                None
            );
        }
        assert!(prefs.people_sides.is_empty());
    }

    #[test]
    fn old_people_wait_for_a_new_meeting_and_aliases_share_history() {
        let mut meetings = meetings();
        assert_eq!(meetings.len(), 1);
        let teams = Teams {
            teams: vec![super::super::Team {
                id: String::new(),
                name: "Design".into(),
                members: vec!["Alex Chen".into()],
                aliases: BTreeMap::from([("Alex C".into(), "Alex Chen".into())]),
            }],
        };
        let mut prefs = UiPrefs::default();
        assert!(initialize(&mut prefs, &meetings));
        assert!(!initialize(&mut prefs, &[]));
        assert!(roster(&meetings, &teams, &[], &prefs, &Affiliations::default()).is_empty());
        let mut new = meetings[0].clone();
        new.id = "101-1".into();
        new.started = Some(chrono::Local::now());
        new.summary.people = vec!["Alex Chen".into(), "Alex C".into(), "New Person".into()];
        new.summary.todo.push(super::super::Todo {
            owner: Some("Old Only".into()),
            ..Default::default()
        });
        meetings.push(new);
        let mut late_old = meetings[0].clone();
        late_old.id = "late-old".into();
        late_old.summary.people = vec!["Late Old".into()];
        meetings.push(late_old);
        let people = roster(&meetings, &teams, &[], &prefs, &Affiliations::default());
        assert_eq!(people.len(), 2);
        assert_eq!(people[0].name, "Alex Chen");
        assert_eq!(people[0].meetings, 2);
        assert_eq!(people[0].aliases, ["Alex C"]);
        assert_eq!(people[0].side, Some(Side::OurTeam));
        prefs.people_sides.insert(key("Alex Chen"), Side::External);
        assert_eq!(
            roster(&meetings, &teams, &[], &prefs, &Affiliations::default())[0].side,
            Some(Side::External)
        );
        let restored: UiPrefs =
            serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert_eq!(restored, prefs);
    }
}

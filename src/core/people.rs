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

/// Automatic evidence is projected, never stored in the manual `people_sides` map.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Affiliations {
    pub your_name: String,
    pub company: Option<String>,
    members: BTreeSet<String>,
    pub notice: Option<String>,
}
impl Affiliations {
    pub fn load(config: &Config) -> Self {
        let kb = &config.knowledge_base;
        let mut result = Self {
            your_name: config
                .notes
                .as_ref()
                .map(|n| n.your_name.clone())
                .unwrap_or_default(),
            company: (!kb.company_name.trim().is_empty())
                .then(|| kb.company_name.trim().to_owned()),
            ..Default::default()
        };
        if kb.people_roster_source.is_empty() {
            return result;
        }
        let read = || -> std::io::Result<String> {
            kb.validate().map_err(super::store_io::invalid)?;
            let bytes = super::daily_check::git(
                &kb.repo_path,
                &[
                    "show",
                    &format!("{}:{}", kb.revision, kb.people_roster_source),
                ],
            )?;
            String::from_utf8(bytes).map_err(|_| super::store_io::invalid("Roster must be UTF-8"))
        };
        match read() {
            Ok(text) => match Self::parse_members(&text) {
                Some(members) => result.members = members,
                None => result.notice = Some("Could not read the team roster. Use a Markdown table with a Person or Name column.".into()),
            },
            Err(_) => result.notice = Some("Could not read the configured team roster. Check the knowledge base repository, revision and roster path.".into()),
        }
        result
    }

    fn parse_members(text: &str) -> Option<BTreeSet<String>> {
        let mut column = None;
        let mut members = BTreeSet::new();
        for line in text.lines() {
            let line = line.trim();
            if !line.starts_with('|') || !line.ends_with('|') {
                column = None;
                continue;
            }
            let cells: Vec<_> = line.trim_matches('|').split('|').map(str::trim).collect();
            if let Some(index) = cells
                .iter()
                .position(|cell| matches!(key(cell).as_str(), "person" | "name"))
            {
                column = Some(index);
                continue;
            }
            if let Some(cell) = column.and_then(|index| cells.get(index)) {
                if cell.chars().all(|c| matches!(c, '-' | ':' | ' ')) {
                    continue;
                }
                // Only explicit spellings in the name column count; roles and prose do not.
                for name in cell.split(['/', ',', '(', ')']) {
                    let name = key(name.trim().trim_matches(['*', '`']));
                    if named(&name) {
                        members.insert(name);
                    }
                }
            }
        }
        (!members.is_empty()).then_some(members)
    }

    pub fn is_self(&self, name: &str, teams: &Teams) -> bool {
        let name = key(&teams.owner_name(name));
        let owner = key(&teams.owner_name(&self.your_name));
        if matches!(name.as_str(), "you" | "me") || (!owner.is_empty() && name == owner) {
            return true;
        }
        // Summaries use short names. Accept one only when the roster lists that exact
        // spelling and has no competing person with the owner's first name.
        owner.split_whitespace().next().is_some_and(|first| {
            name == first
                && self.members.contains(&name)
                && !self.members.iter().any(|member| {
                    member != &name
                        && member != &owner
                        && member.split_whitespace().next() == Some(first)
                })
        })
    }

    pub fn company_for(&self, name: &str, teams: &Teams) -> Option<&str> {
        (self.is_self(name, teams)
            || self.members.contains(&key(name))
            || self.members.contains(&key(&teams.owner_name(name))))
        .then_some(self.company.as_deref())
        .flatten()
    }

    pub fn side(&self, name: &str, teams: &Teams, prefs: &UiPrefs) -> Option<Side> {
        prefs
            .people_sides
            .get(&key(name))
            .copied()
            .or_else(|| {
                prefs
                    .people_sides
                    .get(&key(&teams.owner_name(name)))
                    .copied()
            })
            .or_else(|| {
                (self.is_self(name, teams)
                    || self.members.contains(&key(name))
                    || self.members.contains(&key(&teams.owner_name(name)))
                    || teams.resolve(Some(name), None) != "No team")
                    .then_some(Side::OurTeam)
            })
    }
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
                company: affiliations.company_for(&name, teams).map(str::to_owned),
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
                "# Team\n\n| Person | Role |\n|---|---|\n| Avery Reed / Avery R (A. Reed) | Engineer |\n| Blake Bell, Drew Day | Designer |\n\nOutside contact: Jordan Guest",
            ).unwrap(),
            notice: None,
        }
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
        evidence.members.insert(key("Casey"));
        assert!(evidence.is_self("CASEY", &Teams::default()));
        assert!(!evidence.is_self("Case", &Teams::default()));
        evidence.members.insert(key("Casey Bell"));
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

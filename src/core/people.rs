//! Display-time roster from meeting names and explicit team aliases. No historical writes.
use super::{Meeting, TaskItem, Teams, UiPrefs};
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    OurTeam,
    External,
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
}

pub fn roster(
    meetings: &[Meeting],
    teams: &Teams,
    tasks: &[TaskItem],
    prefs: &UiPrefs,
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
            let side = prefs
                .people_sides
                .get(&id)
                .copied()
                .or_else(|| (team != "No team").then_some(Side::OurTeam));
            let person = people.entry(id).or_insert_with(|| Person {
                aliases: teams
                    .teams
                    .iter()
                    .flat_map(|t| t.aliases.iter())
                    .filter(|(_, target)| key(target) == key(&name))
                    .map(|(alias, _)| alias.clone())
                    .collect(),
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
        let root = std::env::temp_dir().join(format!("sidenote-people-{}", std::process::id()));
        let folder = root.join("100-1");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(
            folder.join("english-summary.json"),
            r#"{"title":"Old meeting","people":["Old Only","Alex C","You"]}"#,
        )
        .unwrap();
        let (meetings, _) = super::super::scan_in(&root);
        std::fs::remove_dir_all(root).unwrap();
        meetings
    }
    #[test]
    fn old_people_wait_for_a_new_meeting_and_aliases_share_history() {
        let mut meetings = meetings();
        assert_eq!(meetings.len(), 1);
        let teams = Teams {
            teams: vec![super::super::Team {
                name: "Design".into(),
                members: vec!["Alex Chen".into()],
                aliases: BTreeMap::from([("Alex C".into(), "Alex Chen".into())]),
            }],
        };
        let mut prefs = UiPrefs::default();
        assert!(initialize(&mut prefs, &meetings));
        assert!(!initialize(&mut prefs, &[]));
        assert!(roster(&meetings, &teams, &[], &prefs).is_empty());
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
        let people = roster(&meetings, &teams, &[], &prefs);
        assert_eq!(people.len(), 2);
        assert_eq!(people[0].name, "Alex Chen");
        assert_eq!(people[0].meetings, 2);
        assert_eq!(people[0].aliases, ["Alex C"]);
        assert_eq!(people[0].side, Some(Side::OurTeam));
        prefs.people_sides.insert(key("Alex Chen"), Side::External);
        assert_eq!(
            roster(&meetings, &teams, &[], &prefs)[0].side,
            Some(Side::External)
        );
        let restored: UiPrefs =
            serde_json::from_slice(&serde_json::to_vec(&prefs).unwrap()).unwrap();
        assert_eq!(restored, prefs);
    }
}

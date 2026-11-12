//! Explicit company identities and person links, published together in one journal transaction.
use super::{Meeting, TaskItem, Teams, UiPrefs, people, store_io, sync_state};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io,
    path::Path,
};

const STORE: &str = "companies.json";

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Company {
    pub id: String,
    pub name: String,
    pub domain: Option<String>,
    pub notes: Option<String>,
    pub aliases: Vec<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PersonLink {
    pub company_id: Option<String>,
    pub email: Option<String>,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Catalog {
    pub records: BTreeMap<String, Company>,
    pub people: BTreeMap<String, PersonLink>,
    pub migrated: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Undo {
    pub label: String,
    pub before: Catalog,
    pub after: Catalog,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Companies {
    pub catalog: Catalog,
    pub undo: Option<Undo>,
}
impl Companies {
    pub fn load(root: &Path) -> io::Result<Self> {
        sync_state::load(root, STORE)
    }
    pub fn find(&self, name: &str) -> Option<&Company> {
        let key = people::key(name);
        self.catalog.records.values().find(|c| {
            people::key(&c.name) == key || c.aliases.iter().any(|a| people::key(a) == key)
        })
    }
    pub fn company_for(&self, name: &str) -> Option<&Company> {
        self.catalog
            .people
            .get(&people::key(name))?
            .company_id
            .as_ref()
            .and_then(|id| self.catalog.records.get(id))
    }
    /// Only explicit, previously saved company text is imported. Meetings are never an input.
    pub fn migrate(root: &Path) -> io::Result<Self> {
        let current = Self::load(root)?;
        if current.catalog.migrated {
            return Ok(current);
        }
        let prefs: serde_json::Value = store_io::read(&root.join("local/ui.json"))?;
        let legacy: serde_json::Value = store_io::read(&root.join("people.json"))?;
        let mut names = BTreeMap::new();
        let mut emails = BTreeMap::new();
        if let Some(map) = prefs
            .get("people_emails")
            .and_then(serde_json::Value::as_object)
        {
            for (person, email) in map {
                if let Some(email) = email.as_str() {
                    emails.insert(people::key(person), email.to_string());
                }
            }
        }
        if let Some(map) = prefs
            .get("people_companies")
            .and_then(serde_json::Value::as_object)
        {
            for (person, company) in map {
                if let Some(company) = company.as_str() {
                    names.insert(people::key(person), company.to_string());
                }
            }
        }
        if let Some(rows) = legacy.get("people").and_then(serde_json::Value::as_array) {
            for row in rows {
                if let (Some(person), Some(company)) = (
                    row.get("name").and_then(serde_json::Value::as_str),
                    row.get("company").and_then(serde_json::Value::as_str),
                ) {
                    names.insert(people::key(person), company.to_string());
                }
                if let (Some(person), Some(email)) = (
                    row.get("name").and_then(serde_json::Value::as_str),
                    row.get("email").and_then(serde_json::Value::as_str),
                ) {
                    emails.insert(people::key(person), email.to_string());
                }
            }
        }
        // Back up original bytes before the first publication. Keep the source files intact.
        for source in ["local/ui.json", "people.json"] {
            let path = root.join(source);
            let backup = root.join("local/company-migration-backup").join(source);
            if path.try_exists()? && !backup.try_exists()? {
                store_io::atomic_write(
                    &backup,
                    &store_io::read_bounded(&path, super::file_reads::SUMMARY_MAX)?,
                )?;
            }
        }
        sync_state::update_checked(root, STORE, |file: &mut Self| {
            if file.catalog.migrated {
                return Ok(());
            }
            for (person, name) in names {
                if people::key(&name).is_empty() || person.is_empty() {
                    continue;
                }
                let id = match file.find(&name) {
                    Some(c) => c.id.clone(),
                    None => {
                        let id = sync_state::new_id(root)?;
                        file.catalog.records.insert(
                            id.clone(),
                            Company {
                                id: id.clone(),
                                name: name.split_whitespace().collect::<Vec<_>>().join(" "),
                                ..Default::default()
                            },
                        );
                        id
                    }
                };
                file.catalog
                    .people
                    .entry(person)
                    .or_default()
                    .company_id
                    .get_or_insert(id);
            }
            for (person, email) in emails {
                if !person.is_empty() && !email.trim().is_empty() {
                    file.catalog
                        .people
                        .entry(person)
                        .or_default()
                        .email
                        .get_or_insert(email);
                }
            }
            file.catalog.migrated = true;
            Ok(())
        })
    }
    pub fn save_company(root: &Path, mut company: Company) -> io::Result<Self> {
        company.name = company
            .name
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if company.name.is_empty() {
            return Err(store_io::invalid("Enter a company name."));
        }
        if company.id.is_empty() {
            company.id = sync_state::new_id(root)?;
        }
        Self::change(root, "Edit company", move |file| {
            if file.catalog.records.values().any(|c| {
                c.id != company.id
                    && std::iter::once(&company.name)
                        .chain(&company.aliases)
                        .any(|name| {
                            people::key(&c.name) == people::key(name)
                                || c.aliases
                                    .iter()
                                    .any(|a| people::key(a) == people::key(name))
                        })
            }) {
                return Err(store_io::invalid(
                    "That name or alias belongs to another company. Merge them instead.",
                ));
            }
            if let Some(old) = file.catalog.records.get(&company.id)
                && people::key(&old.name) != people::key(&company.name)
            {
                company.aliases.push(old.name.clone());
            }
            let mut seen = BTreeSet::from([people::key(&company.name)]);
            company
                .aliases
                .retain(|a| !people::key(a).is_empty() && seen.insert(people::key(a)));
            company.domain = company.domain.filter(|s| !s.trim().is_empty());
            company.notes = company.notes.filter(|s| !s.trim().is_empty());
            file.catalog.records.insert(company.id.clone(), company);
            Ok(())
        })
    }
    pub fn link(
        root: &Path,
        person: &str,
        company_id: Option<String>,
        email: Option<String>,
    ) -> io::Result<Self> {
        let person = people::key(person);
        Self::change(root, "Edit person company", move |file| {
            if person.is_empty()
                || company_id
                    .as_ref()
                    .is_some_and(|id| !file.catalog.records.contains_key(id))
            {
                return Err(store_io::invalid("Choose an existing person and company."));
            }
            file.catalog.people.insert(
                person,
                PersonLink {
                    company_id,
                    email: email.filter(|s| !s.trim().is_empty()),
                },
            );
            Ok(())
        })
    }
    pub fn merge(root: &Path, from: &str, into: &str) -> io::Result<Self> {
        Self::change(root, "Merge companies", |file| {
            if from == into
                || !file.catalog.records.contains_key(from)
                || !file.catalog.records.contains_key(into)
            {
                return Err(store_io::invalid("Choose two different companies."));
            }
            let source = file.catalog.records.remove(from).unwrap();
            let target = file.catalog.records.get_mut(into).unwrap();
            target.aliases.push(source.name);
            target.aliases.extend(source.aliases);
            if let Some(domain) = source.domain {
                if target.domain.is_none() {
                    target.domain = Some(domain);
                } else if target.domain.as_ref() != Some(&domain) {
                    target.notes = Some(
                        format!(
                            "{}\nMerged domain: {}",
                            target.notes.as_deref().unwrap_or_default(),
                            domain
                        )
                        .trim()
                        .into(),
                    );
                }
            }
            if let Some(notes) = source.notes {
                target.notes = Some(
                    format!("{}\n{}", target.notes.as_deref().unwrap_or_default(), notes)
                        .trim()
                        .into(),
                );
            }
            let mut seen = BTreeSet::from([people::key(&target.name)]);
            target.aliases.retain(|a| seen.insert(people::key(a)));
            for person in file.catalog.people.values_mut() {
                if person.company_id.as_deref() == Some(from) {
                    person.company_id = Some(into.into());
                }
            }
            Ok(())
        })
    }
    fn change(
        root: &Path,
        label: &str,
        change: impl FnOnce(&mut Self) -> io::Result<()>,
    ) -> io::Result<Self> {
        sync_state::update_checked(root, STORE, |file: &mut Self| {
            let before = file.catalog.clone();
            change(file)?;
            if file.catalog != before {
                file.undo = Some(Undo {
                    label: label.into(),
                    before,
                    after: file.catalog.clone(),
                });
            }
            Ok(())
        })
    }
    pub fn undo(root: &Path) -> io::Result<Self> {
        sync_state::update_checked(root, STORE, |file: &mut Self| {
            let undo = file
                .undo
                .as_ref()
                .ok_or_else(|| store_io::invalid("Nothing to undo."))?;
            if file.catalog != undo.after {
                return Err(store_io::invalid(
                    "Company data changed after this edit. Undo would overwrite newer changes.",
                ));
            }
            file.catalog = undo.before.clone();
            file.undo = None;
            Ok(())
        })
    }
}

pub struct CompanyData<'a> {
    pub people: Vec<people::Person>,
    pub meetings: Vec<&'a Meeting>,
    pub open: Vec<&'a TaskItem>,
    pub ours: Vec<&'a TaskItem>,
    pub decisions: Vec<(&'a Meeting, &'a super::Point)>,
}
pub struct DecisionGroup<'a> {
    pub key: String,
    pub text: &'a str,
    pub citations: Vec<&'a Meeting>,
}

/// Keep the query's newest-first order and one citation per source meeting.
pub fn group_decisions<'a>(
    decisions: &[(&'a Meeting, &'a super::Point)],
) -> Vec<DecisionGroup<'a>> {
    let mut groups: Vec<DecisionGroup<'a>> = Vec::new();
    let mut positions = BTreeMap::new();
    for (meeting, point) in decisions {
        let key = people::key(point.text.trim().trim_end_matches('.').trim());
        if key.is_empty() {
            continue;
        }
        let index = *positions.entry(key.clone()).or_insert_with(|| {
            groups.push(DecisionGroup {
                key,
                text: &point.text,
                citations: Vec::new(),
            });
            groups.len() - 1
        });
        let citations = &mut groups[index].citations;
        if !citations
            .iter()
            .any(|m| m.id == meeting.id && m.device == meeting.device)
        {
            citations.push(meeting);
        }
    }
    groups
}
impl CompanyData<'_> {
    pub fn ask_context(
        &self,
        tasks: &[TaskItem],
        pipeline: &super::Pipeline,
        projects: &super::Projects,
    ) -> super::ask_all::AllContext {
        let meetings: Vec<_> = self.meetings.iter().map(|m| (*m).clone()).collect();
        let tasks: Vec<_> = tasks
            .iter()
            .filter(|t| meetings.iter().any(|m| m.id == t.meeting_id))
            .cloned()
            .collect();
        super::ask_all::all_context(
            &meetings,
            &tasks,
            pipeline,
            projects,
            chrono::Local::now().date_naive(),
            super::ask_all::CONTEXT_CAP,
        )
    }
}
pub fn query<'a>(
    id: &str,
    companies: &Companies,
    meetings: &'a [Meeting],
    teams: &Teams,
    tasks: &'a [TaskItem],
    prefs: &UiPrefs,
    affiliations: &people::Affiliations,
) -> CompanyData<'a> {
    let roster = people::roster(meetings, teams, tasks, prefs, affiliations);
    let people: Vec<_> = roster
        .iter()
        .filter(|p| companies.company_for(&p.name).is_some_and(|c| c.id == id))
        .map(|p| people::key(&p.name))
        .collect();
    let mut found: Vec<_> = meetings
        .iter()
        .filter(|m| {
            m.summary
                .people
                .iter()
                .any(|n| people.contains(&people::key(&teams.owner_name(n))))
        })
        .collect();
    found.sort_by_key(|m| std::cmp::Reverse(m.started));
    let open = tasks
        .iter()
        .filter(|t| {
            !t.done
                && t.owner
                    .as_ref()
                    .is_some_and(|n| people.contains(&people::key(&teams.owner_name(n))))
        })
        .collect();
    let ours = tasks
        .iter()
        .filter(|t| {
            !t.done
                && found.iter().any(|m| m.id == t.meeting_id)
                && t.owner.as_ref().is_some_and(|n| {
                    roster.iter().any(|p| {
                        p.side == Some(people::Side::OurTeam)
                            && people::key(&p.name) == people::key(&teams.owner_name(n))
                    })
                })
        })
        .collect();
    let decisions = found
        .iter()
        .flat_map(|m| m.points().filter(|p| p.kind == "decided").map(|p| (*m, p)))
        .collect();
    CompanyData {
        people: roster
            .into_iter()
            .filter(|p| people.contains(&people::key(&p.name)))
            .collect(),
        meetings: found,
        open,
        ours,
        decisions,
    }
}

#[cfg(test)]
#[path = "companies_tests.rs"]
mod tests;

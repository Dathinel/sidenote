use super::*;
use std::fs;

struct Fixture(std::path::PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "companies-{}-{}",
            std::process::id(),
            super::super::device::random_id().unwrap()
        ));
        fs::create_dir_all(root.join("local")).unwrap();
        Self(root)
    }
    fn company(&self, name: &str) -> Company {
        Companies::save_company(
            &self.0,
            Company {
                name: name.into(),
                ..Default::default()
            },
        )
        .unwrap()
        .find(name)
        .unwrap()
        .clone()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn migration_is_explicit_backed_up_deduplicated_and_restart_safe() {
    let f = Fixture::new();
    let original = br#"{"people_companies":{"Aster Vale":"  Northwind   Labs ","Briar West":"NORTHWIND labs","Old Only":"","Cedar Ray":"Lumen"}}"#;
    fs::write(f.0.join("local/ui.json"), original).unwrap();
    fs::write(
        f.0.join("people.json"),
        br#"{"people":[{"name":"Delta Moss","company":"northwind labs","email":"delta@example.test"}]}"#,
    )
    .unwrap();
    let first = Companies::migrate(&f.0).unwrap();
    assert_eq!(first.catalog.records.len(), 2);
    assert_eq!(first.catalog.people.len(), 4);
    assert_eq!(
        first.catalog.people["delta moss"].email.as_deref(),
        Some("delta@example.test")
    );
    assert_eq!(
        first.company_for("aster vale").unwrap().id,
        first.company_for("Briar West").unwrap().id
    );
    assert_eq!(
        fs::read(f.0.join("local/company-migration-backup/local/ui.json")).unwrap(),
        original
    );
    fs::write(
        f.0.join("local/ui.json"),
        br#"{"people_companies":{"Invented":"Never import later"}}"#,
    )
    .unwrap();
    assert_eq!(Companies::migrate(&f.0).unwrap(), first);
    assert_eq!(Companies::load(&f.0).unwrap(), first);
}
#[test]
fn corrupt_migration_source_keeps_inputs_and_does_not_publish() {
    let f = Fixture::new();
    fs::write(f.0.join("people.json"), b"broken").unwrap();
    assert!(Companies::migrate(&f.0).is_err());
    assert!(!Companies::load(&f.0).unwrap().catalog.migrated);
    assert_eq!(fs::read(f.0.join("people.json")).unwrap(), b"broken");
}
#[test]
fn rename_dedupe_merge_and_undo_retain_person_links_and_optional_fields() {
    let f = Fixture::new();
    let mut north = f.company("Northwind");
    let mut lumen = f.company("Lumen");
    north.domain = Some("northwind.example".into());
    north.notes = Some("Keep the prototype".into());
    Companies::save_company(&f.0, north.clone()).unwrap();
    lumen.domain = Some("lumen.example".into());
    Companies::save_company(&f.0, lumen.clone()).unwrap();
    assert!(
        Companies::save_company(
            &f.0,
            Company {
                name: " NORTHWIND ".into(),
                ..Default::default()
            }
        )
        .is_err()
    );
    Companies::link(
        &f.0,
        "Aster Vale",
        Some(north.id.clone()),
        Some("aster@example.test".into()),
    )
    .unwrap();
    north.name = "Northwind Labs".into();
    let renamed = Companies::save_company(&f.0, north.clone()).unwrap();
    assert_eq!(renamed.find("northwind").unwrap().name, "Northwind Labs");
    assert_eq!(
        Companies::undo(&f.0)
            .unwrap()
            .find("Northwind")
            .unwrap()
            .name,
        "Northwind"
    );
    let before = Companies::load(&f.0).unwrap().catalog;
    let merged = Companies::merge(&f.0, &north.id, &lumen.id).unwrap();
    assert_eq!(merged.catalog.records.len(), 1);
    assert_eq!(merged.company_for("aster vale").unwrap().id, lumen.id);
    assert!(
        merged
            .find("Northwind")
            .unwrap()
            .notes
            .as_ref()
            .unwrap()
            .contains("northwind.example")
    );
    assert_eq!(Companies::undo(&f.0).unwrap().catalog, before);
    assert_eq!(Companies::load(&f.0).unwrap().catalog, before);
    assert!(Companies::undo(&f.0).is_err());
    assert!(Companies::merge(&f.0, &north.id, &north.id).is_err());
    assert!(Companies::merge(&f.0, "missing", &north.id).is_err());
}
#[test]
fn undo_refuses_to_overwrite_an_intervening_change() {
    let f = Fixture::new();
    let north = f.company("Northwind");
    sync_state::update_checked(&f.0, STORE, |file: &mut Companies| {
        file.catalog.records.get_mut(&north.id).unwrap().notes = Some("Newer peer edit".into());
        Ok(())
    })
    .unwrap();
    assert!(Companies::undo(&f.0).is_err());
    assert_eq!(
        Companies::load(&f.0)
            .unwrap()
            .find("Northwind")
            .unwrap()
            .notes
            .as_deref(),
        Some("Newer peer edit")
    );
}

#[test]
fn company_ask_threads_survive_restart_without_sharing_global_or_other_company_history() {
    let f = Fixture::new();
    let store = super::super::StateStore::new(f.0.clone());
    let key = super::super::company_ask_key("Northwind/../../");
    store
        .append_message(
            &key,
            super::super::Message {
                role: "user".into(),
                text: "Where do things stand?".into(),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(store.load(&key).unwrap().messages.len(), 1);
    assert!(
        store
            .load(super::super::ALL_KEY)
            .unwrap()
            .messages
            .is_empty()
    );
    assert!(
        store
            .load(&super::super::company_ask_key("Lumen"))
            .unwrap()
            .messages
            .is_empty()
    );
    assert!(store.load("company-../../outside").is_err());
}
#[test]
fn tab_queries_use_confirmed_links_and_aliases_without_exposing_old_only_people() {
    let f = Fixture::new();
    let north = f.company("Northwind");
    Companies::link(&f.0, "Aster Vale", Some(north.id.clone()), None).unwrap();
    Companies::link(&f.0, "Old Only", Some(north.id.clone()), None).unwrap();
    for (id, names, at) in [
        ("100-1", vec!["Old Only"], 1000),
        (
            "101-1",
            vec!["A Vale", "Aster Vale", "Ours"],
            chrono::Local::now().timestamp_millis(),
        ),
    ] {
        let dir = f.0.join("meetings").join(id);
        fs::create_dir_all(&dir).unwrap();
        store_io::write(&dir.join("english-summary.json"), &serde_json::json!({"title":id,"people":names,"topics":[{"title":"Plan","points":[{"type":"decided","text":"Use the small prototype"},{"type":"discussed","text":"Not a decision"}]}],"todo":[{"id":"followup","task":"Confirm prototype","owner":"A Vale"},{"id":"ours","task":"Prepare prototype","owner":"Ours"}]})).unwrap();
        store_io::write(
            &dir.join("manifest.json"),
            &serde_json::json!({"created_at_ms":at}),
        )
        .unwrap();
    }
    let (meetings, errors) = super::super::scan_in(&f.0.join("meetings"));
    assert!(errors.is_empty());
    let teams = Teams {
        teams: vec![super::super::Team {
            name: "Delivery".into(),
            members: vec!["Ours".into(), "Aster Vale".into()],
            aliases: BTreeMap::from([("A Vale".into(), "Aster Vale".into())]),
        }],
    };
    let mut prefs = UiPrefs {
        people_existing_meetings: Some(BTreeSet::from(["local/100-1".into()])),
        people_since_ms: Some(2000),
        ..Default::default()
    };
    prefs
        .people_sides
        .insert("aster vale".into(), people::Side::External);
    let tasks = super::super::build_tasks(
        &meetings,
        &Default::default(),
        &Default::default(),
        &Default::default(),
        &Default::default(),
    );
    let companies = Companies::load(&f.0).unwrap();
    let data = query(&north.id, &companies, &meetings, &teams, &tasks, &prefs);
    assert_eq!(data.people.len(), 1);
    assert_eq!(data.people[0].name, "Aster Vale");
    assert_eq!(data.meetings.len(), 1);
    assert_eq!(data.meetings[0].id, "101-1");
    assert_eq!(data.decisions.len(), 1);
    assert_eq!(data.decisions[0].1.text, "Use the small prototype");
    assert_eq!(data.open.len(), 2); // Same explicit owner on both source summaries.
    assert_eq!(data.ours.len(), 1);
    let context = data.ask_context(&tasks, &Default::default(), &Default::default());
    assert_eq!(context.included, 1);
    assert!(!context.text.contains("100-1"));
    let empty = query("missing", &companies, &meetings, &teams, &tasks, &prefs);
    assert_eq!(
        empty
            .ask_context(&tasks, &Default::default(), &Default::default())
            .included,
        0
    );
    assert!(
        empty.people.is_empty()
            && empty.meetings.is_empty()
            && empty.decisions.is_empty()
            && empty.open.is_empty()
            && empty.ours.is_empty()
    );
}

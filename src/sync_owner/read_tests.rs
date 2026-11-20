use super::*;
use serde_json::json;

#[test]
fn large_inventory_stays_readable_and_cursors_detect_changes() -> io::Result<()> {
    // Keep the inventory above the frame cap with fewer pages to hash.
    let revision = "a".repeat(14 * 1024);
    let mut state = Snapshot {
        meetings: (0..80)
            .map(|n| Ingested {
                device: Some("d".repeat(32)),
                id: format!("{n}-1"),
                revision: revision.clone(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    };
    assert!(serde_json::to_vec(&state)?.len() > MAX_REPLY);
    assert!(serde_json::to_vec(&pages::status(&state))?.len() < 1024);
    let first = pages::page(&state.meetings, None)?;
    let stale = first["next"].clone();
    let mut page = first;
    let mut seen = std::collections::BTreeSet::new();
    loop {
        assert!(serde_json::to_vec(&page)?.len() < MAX_REPLY);
        for row in page["items"].as_array().unwrap() {
            assert!(seen.insert(row["id"].as_str().unwrap().to_owned()));
        }
        if page["next"].is_null() {
            break;
        }
        page = pages::page(
            &state.meetings,
            Some(serde_json::from_value(page["next"].clone())?),
        )?;
    }
    assert_eq!(seen.len(), state.meetings.len());
    state.meetings[0].revision.replace_range(..1, "b");
    assert!(
        pages::page(&state.meetings, Some(serde_json::from_value(stale)?))
            .unwrap_err()
            .to_string()
            .contains("CURSOR_STALE")
    );
    assert!(
        pages::page(&["x".repeat(32 * 1024)], None)
            .unwrap_err()
            .to_string()
            .contains("ITEM_TOO_LARGE")
    );
    Ok(())
}
#[test]
fn many_conflict_choices_have_a_small_selector_and_bounded_pages() -> io::Result<()> {
    let conflict = relay::Conflict {
        domain: "tasks".into(),
        task_id: "synthetic".into(),
        title: String::new(),
        field: "owner".into(),
        choices: (0..80)
            .map(|n| relay::Choice {
                value: json!(format!("{n}:{}", "x".repeat(14 * 1024))),
                device: "fixture".into(),
                at: String::new(),
                source: n.to_string(),
            })
            .collect(),
    };
    assert!(serde_json::to_vec(&conflict)?.len() > MAX_REPLY);
    let selector = pages::conflict_ref(&conflict)?;
    assert!(serde_json::to_vec(&selector)?.len() < 512);
    let mut page = pages::page(&conflict.choices, None)?;
    let mut count = 0;
    loop {
        assert!(serde_json::to_vec(&page)?.len() < MAX_REPLY);
        count += page["items"].as_array().unwrap().len();
        if page["next"].is_null() {
            break;
        }
        page = pages::page(
            &conflict.choices,
            Some(serde_json::from_value(page["next"].clone())?),
        )?;
    }
    assert_eq!(count, conflict.choices.len());
    Ok(())
}

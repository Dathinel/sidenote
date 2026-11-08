//! Synthetic contract tests. Compiled locally by clippy; executed only in hosted CI.
use chrono::{DateTime, NaiveDate, Utc};
use meetings_domain::{
    ids::{NS_MEETING, NS_THREAD},
    *,
};
use proptest::prelude::*;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashSet};
use uuid::Uuid;

fn id<E>() -> Id<E> {
    Id::from_uuid(Uuid::from_u128(0x019b0000000070008000000000000001))
}
fn at() -> DateTime<Utc> {
    DateTime::from_timestamp(1_800_000_000, 0).unwrap()
}
fn blob() -> BlobId {
    BlobId([42; 32])
}
fn provenance() -> Provenance {
    Provenance {
        harness: Some("synthetic-harness".into()),
        harness_version: Some("0.1-test".into()),
        agent_name: Some("Synthetic assistant".into()),
        task_kind: Some("ask".into()),
        duration_ms: Some(42),
        tools_used: vec!["synthetic_read".into()],
        source: Some("synthetic fixture".into()),
        context: Context {
            meeting_ids: vec![id()],
            capped: true,
        },
        ..Provenance::from_models(
            Some("synthetic-reported".into()),
            Some("synthetic-requested".into()),
        )
    }
}

fn round_trip<E: Entity + Eq>(payload: E, deleted: bool) {
    let mut record = Envelope::new(id(), id(), at(), Actor::Device(id()), payload);
    record.deleted = deleted;
    record.provenance = Some(provenance());
    record.updated_at = at() + chrono::Duration::seconds(3);
    let versioned = Versioned {
        version: Version("opaque-engine-token".into()),
        entity: record,
    };
    let wire = serde_json::to_value(&versioned).unwrap();
    assert_eq!(wire["entity"]["schema"], 1);
    assert_eq!(wire["entity"]["deleted"], deleted);
    assert_eq!(
        serde_json::from_value::<Versioned<E>>(wire.clone()).unwrap(),
        versioned
    );
    let mut newer = wire["entity"].clone();
    newer["schema"] = json!(2);
    newer["data"] = json!("new shape unknown to this reader");
    assert!(
        serde_json::from_value::<Envelope<E>>(newer)
            .unwrap_err()
            .to_string()
            .contains("Update Meetings on this device")
    );
    let mut unsupported = wire["entity"].clone();
    unsupported["schema"] = json!(0);
    assert!(serde_json::from_value::<Envelope<E>>(unsupported).is_err());
}

/// Exercise every ADR 0001 entity with both live records and permanent tombstones.
fn all_entities(text: &str, deleted: bool) {
    round_trip(
        Workspace {
            name: text.into(),
            kind: WorkspaceKind::Personal,
        },
        deleted,
    );
    round_trip(
        Meeting {
            legacy_key: "1800000000000-1".into(),
            device_id: id(),
            title: text.into(),
            started_at: at(),
            duration_ms: Some(120_000),
            project_id: Some(id()),
            attendee_ids: vec![id()],
        },
        deleted,
    );
    round_trip(
        Transcript {
            meeting_id: id(),
            language: Some("en".into()),
            body: blob(),
        },
        deleted,
    );
    round_trip(
        Note {
            meeting_id: id(),
            kind: NoteKind::GeneratedSummary,
            body: blob(),
        },
        deleted,
    );
    round_trip(
        Task {
            meeting_id: Some(id()),
            project_id: Some(id()),
            text: text.into(),
            owner_id: Some(id()),
            due: Some(NaiveDate::from_ymd_opt(2027, 1, 15).unwrap()),
            completed: true,
        },
        deleted,
    );
    round_trip(Project { name: text.into() }, deleted);
    round_trip(
        MemoryRecord {
            kind: MemoryKind::Decision,
            body: blob(),
            status: MemoryStatus::Active,
            project_id: Some(id()),
            people: vec![id()],
            sources: vec![
                MemorySource::Meeting {
                    meeting_id: id(),
                    at_ms: Some(3_000),
                },
                MemorySource::KnowledgeBase {
                    path: "synthetic/decision.md".into(),
                    revision: "a".repeat(40),
                },
            ],
            supersedes: Some(id()),
        },
        deleted,
    );
    round_trip(
        ChatThread {
            title: text.into(),
            title_source: TitleSource::User,
            scope: ChatScope::Meeting(id()),
            pinned: true,
            archived: false,
        },
        deleted,
    );
    round_trip(
        ChatMessage {
            thread_id: id(),
            role: ChatRole::Assistant,
            text: TextContent::inline(text.into()).unwrap(),
            device_id: id(),
            status: MessageStatus::Complete,
            citations: vec![Citation {
                meeting_id: id(),
                at_ms: Some(42),
            }],
        },
        deleted,
    );
    round_trip(
        Person {
            name: text.into(),
            email: Some("person@example.invalid".into()),
        },
        deleted,
    );
    round_trip(
        AttendeeSuggestion {
            meeting_id: id(),
            person_id: Some(id()),
            name: text.into(),
            evidence: AttendeeEvidence::TranscriptSelfIntroduction {
                transcript_id: id(),
                at_ms: 42,
            },
        },
        deleted,
    );
    round_trip(
        Device {
            name: text.into(),
            legacy_key: Some("synthetic-device-a".into()),
        },
        deleted,
    );
    round_trip(
        DailyReport {
            date: NaiveDate::from_ymd_opt(2027, 1, 15).unwrap(),
            body: blob(),
            meeting_ids: vec![id()],
            proposal_ids: vec![id()],
        },
        deleted,
    );
    round_trip(
        Proposal::pending(ProposalKind::NewProject { name: text.into() }),
        deleted,
    );
}

#[test]
fn every_entity_round_trips() {
    for deleted in [false, true] {
        all_entities("Synthetic project planning", deleted);
    }
}

#[derive(Deserialize)]
struct MeetingVector {
    device: String,
    folder: String,
    uuid: String,
}

#[test]
fn meeting_ids_match_independent_golden_vectors() {
    // Computed using Python's standard-library uuid.uuid5, not the implementation under test.
    let vectors: Vec<MeetingVector> =
        serde_json::from_str(include_str!("fixtures/meeting-ids.json")).unwrap();
    for vector in vectors {
        let actual = meeting_id(&vector.device, &vector.folder).unwrap();
        assert_eq!(actual.to_string(), vector.uuid);
        assert_eq!(actual.as_uuid().get_version_num(), 5);
    }
}

#[test]
fn meeting_key_components_cannot_alias() {
    for (device, folder) in [
        ("", "1800000000000-1"),
        ("a/b", "1800000000000-1"),
        ("a", "b/1800000000000-1"),
        ("a", "-1"),
        ("a", "1-"),
        ("a", "1-2-3"),
        ("a", "abc-1"),
    ] {
        assert!(meeting_id(device, folder).is_err());
    }
}

#[test]
fn namespace_contracts_are_fixed_and_distinct() {
    let namespaces = [
        Workspace::NAMESPACE,
        Meeting::NAMESPACE,
        Transcript::NAMESPACE,
        Note::NAMESPACE,
        Task::NAMESPACE,
        Project::NAMESPACE,
        MemoryRecord::NAMESPACE,
        ChatThread::NAMESPACE,
        ChatMessage::NAMESPACE,
        Person::NAMESPACE,
        AttendeeSuggestion::NAMESPACE,
        Device::NAMESPACE,
        DailyReport::NAMESPACE,
        Proposal::NAMESPACE,
    ];
    let names = [
        "workspace",
        "meeting",
        "transcript",
        "note",
        "task",
        "project",
        "memory_record",
        "chat_thread",
        "chat_message",
        "person",
        "attendee_suggestion",
        "device",
        "daily_report",
        "proposal",
    ];
    assert_eq!(namespaces.into_iter().collect::<HashSet<_>>().len(), 14);
    for (namespace, kind) in namespaces.into_iter().zip(names) {
        assert_eq!(
            namespace,
            Uuid::new_v5(
                &Uuid::NAMESPACE_URL,
                format!("https://hex-meetings.org/ns/{kind}").as_bytes()
            )
        );
    }
    assert_ne!(NS_THREAD, NS_MEETING);
    assert_eq!(
        Id::<ChatThread>::from_key(b"all"),
        Id::new_v5(&NS_THREAD, b"all")
    );
}

#[test]
fn new_ids_are_v7_and_unique_without_payload_trait_bounds() {
    struct NoTraits;
    let ids: BTreeSet<_> = (0..128).map(|_| Id::<NoTraits>::new_v7()).collect();
    assert_eq!(ids.len(), 128);
    for id in ids {
        assert_eq!(id.as_uuid().get_version_num(), 7);
        assert!(id.as_uuid().get_timestamp().is_some());
        assert_eq!(id.to_string().parse::<Id<NoTraits>>().unwrap(), id);
        assert_eq!(
            serde_json::from_str::<Id<NoTraits>>(&serde_json::to_string(&id).unwrap()).unwrap(),
            id
        );
    }
}

#[test]
fn inline_text_checks_utf8_bytes_on_construction_and_deserialization() {
    let boundary = "x".repeat(MAX_INLINE_TEXT_BYTES);
    let content = TextContent::inline(boundary.clone()).unwrap();
    assert_eq!(content.as_inline(), Some(boundary.as_str()));
    assert!(TextContent::inline(format!("{boundary}x")).is_err());
    assert!(TextContent::inline("\u{00e9}".repeat(MAX_INLINE_TEXT_BYTES / 2 + 1)).is_err());
    assert!(
        serde_json::from_value::<TextContent>(
            json!({"kind":"inline", "value":format!("{boundary}x")})
        )
        .is_err()
    );
    let blob_content = TextContent::blob(blob());
    assert_eq!(blob_content.as_blob(), Some(blob()));
    assert_eq!(blob_content.as_inline(), None);
    assert_eq!(
        serde_json::from_value::<TextContent>(serde_json::to_value(&blob_content).unwrap())
            .unwrap(),
        blob_content
    );
}

#[test]
fn every_proposal_kind_starts_pending_and_round_trips() {
    let kinds = [
        ProposalKind::Memory(MemoryProposal {
            operation: MemoryOperation::Add,
            text: TextContent::inline("Synthetic decision".into()).unwrap(),
            kind: MemoryKind::Decision,
            project_id: None,
            people: vec![],
            sources: vec![],
            rationale: "Synthetic evidence".into(),
            expected_version: None,
        }),
        ProposalKind::ChatSummary(ChatSummaryProposal {
            title: "Synthetic conversation".into(),
            summary: TextContent::inline("Synthetic summary".into()).unwrap(),
            key_points: vec!["Point".into()],
            decisions: vec![],
            follow_ups: vec![],
            related_meeting_ids: vec![id()],
            source: ChatSummarySource {
                client_name: "synthetic-client".into(),
                model: None,
                conversation_ref: None,
            },
        }),
        ProposalKind::TaskCompletion {
            task_id: id(),
            expected_version: Some(Version("v1".into())),
        },
        ProposalKind::NewProject {
            name: "Synthetic project".into(),
        },
    ];
    for kind in kinds {
        let proposal = Proposal::pending(kind);
        assert_eq!(proposal.status, ProposalStatus::Pending);
        assert_eq!(proposal.decided_at, None);
        round_trip(proposal, false);
    }
}

#[test]
fn chat_scopes_and_evidence_variants_round_trip() {
    for scope in [
        ChatScope::Meeting(id()),
        ChatScope::AllMeetings,
        ChatScope::Project(id()),
        ChatScope::None,
    ] {
        round_trip(
            ChatThread {
                title: "Synthetic thread".into(),
                title_source: TitleSource::Auto,
                scope,
                pinned: false,
                archived: true,
            },
            false,
        );
    }
    for role in [
        ChatRole::User,
        ChatRole::Assistant,
        ChatRole::ExternalSummary,
        ChatRole::Note,
    ] {
        for status in [
            MessageStatus::Complete,
            MessageStatus::Failed,
            MessageStatus::Stopped,
        ] {
            round_trip(
                ChatMessage {
                    thread_id: id(),
                    role,
                    text: TextContent::blob(blob()),
                    device_id: id(),
                    status,
                    citations: vec![],
                },
                false,
            );
        }
    }
    for evidence in [
        AttendeeEvidence::Calendar {
            event_ref: "synthetic-event".into(),
        },
        AttendeeEvidence::Manual,
    ] {
        round_trip(
            AttendeeSuggestion {
                meeting_id: id(),
                person_id: None,
                name: "Synthetic person".into(),
                evidence,
            },
            false,
        );
    }
    for kind in [NoteKind::UserNotes, NoteKind::Scratchpad] {
        round_trip(
            Note {
                meeting_id: id(),
                kind,
                body: blob(),
            },
            false,
        );
    }
    round_trip(
        Workspace {
            name: "Synthetic team".into(),
            kind: WorkspaceKind::Team,
        },
        false,
    );
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct UpgradedProject {
    title: String,
}
impl Entity for UpgradedProject {
    const KIND: &'static str = "project";
    const NAMESPACE: Uuid = Project::NAMESPACE;
    const SCHEMA: u16 = 2;
    fn upgrade(schema: u16, mut data: Value) -> Result<Value, SchemaError> {
        match schema {
            1 => {
                data["title"] = data["name"].take();
                Ok(data)
            }
            2 => Ok(data),
            found if found > 2 => Err(SchemaError::Newer {
                kind: Self::KIND,
                found,
                supported: 2,
            }),
            found => Err(SchemaError::Unsupported {
                kind: Self::KIND,
                found,
            }),
        }
    }
}

#[test]
fn read_upgrades_write_latest_and_old_reader_refuses_new_schema() {
    let old = Envelope::new(
        id(),
        id(),
        at(),
        Actor::User(Uuid::nil()),
        Project {
            name: "Synthetic project".into(),
        },
    );
    let wire = serde_json::to_value(old).unwrap();
    let new: Envelope<UpgradedProject> = serde_json::from_value(wire).unwrap();
    assert_eq!(new.schema(), 2);
    assert_eq!(new.data.title, "Synthetic project");
    let rewritten = serde_json::to_value(&new).unwrap();
    assert_eq!(rewritten["schema"], 2);
    assert_eq!(
        serde_json::from_value::<Envelope<UpgradedProject>>(rewritten.clone()).unwrap(),
        new
    );
    assert!(
        serde_json::from_value::<Envelope<Project>>(rewritten)
            .unwrap_err()
            .to_string()
            .contains("Update Meetings on this device")
    );
}

#[test]
fn future_schema_is_rejected_before_payload_or_envelope_fields() {
    let error = serde_json::from_value::<Envelope<Meeting>>(json!({"schema":2})).unwrap_err();
    assert!(error.to_string().contains("Update Meetings on this device"));
    for schema in [json!(-1), json!(65536), json!("1"), Value::Null] {
        assert!(serde_json::from_value::<Envelope<Meeting>>(json!({"schema":schema})).is_err());
    }
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 256, failure_persistence: None, ..ProptestConfig::default() })]

    #[test]
    fn arbitrary_unicode_entities_round_trip(text in ".{0,128}", deleted in any::<bool>()) {
        all_entities(&text, deleted);
    }

    #[test]
    fn typed_uuid_wire_round_trip(bits in any::<u128>()) {
        let id = Id::<Meeting>::from_uuid(Uuid::from_u128(bits));
        prop_assert_eq!(id.to_string().parse::<Id<Meeting>>().unwrap(), id);
        prop_assert_eq!(serde_json::from_str::<Id<Meeting>>(&serde_json::to_string(&id).unwrap()).unwrap(), id);
    }

    #[test]
    fn imported_ids_are_deterministic_and_kind_scoped(key in prop::collection::vec(any::<u8>(), 0..256)) {
        let meeting = Id::<Meeting>::from_key(&key);
        let thread = Id::<ChatThread>::from_key(&key);
        prop_assert_eq!(meeting, Id::<Meeting>::new_v5(&NS_MEETING, &key));
        prop_assert_ne!(meeting.as_uuid(), thread.as_uuid());
    }

    #[test]
    fn meeting_identity_includes_device_and_exact_folder(
        device in "[a-z]{1,24}", millis in 0u64..u64::MAX, index in 0u32..u32::MAX
    ) {
        let folder = format!("{millis}-{index}");
        let actual = meeting_id(&device, &folder).unwrap();
        prop_assert_eq!(actual, meeting_id(&device, &folder).unwrap());
        prop_assert_eq!(actual, Id::new_v5(&NS_MEETING, format!("hex/{device}/{folder}").as_bytes()));
        prop_assert_ne!(actual, meeting_id(&format!("{device}-other"), &folder).unwrap());
        prop_assert_ne!(actual, meeting_id(&device, &format!("{millis}-{}", index + 1)).unwrap());
    }
}

//! Markdown for one meeting: what the "..." menu copies, and what Export writes to a file.
//!
//! Every function reads one `Meeting` and returns plain text. Nothing here touches other
//! meetings, and the transcript is never included: it has its own Copy button.
use super::{Meeting, Point};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

fn with_who(point: &Point) -> String {
    match point.who.as_deref().filter(|w| !w.is_empty()) {
        Some(who) => format!("{} ({who})", point.text),
        None => point.text.clone(),
    }
}

/// The decisions as a numbered list. Empty when nothing was decided.
pub fn decisions_markdown(m: &Meeting) -> String {
    m.points()
        .filter(|p| p.kind == "decided")
        .enumerate()
        .map(|(i, p)| format!("{}. {}\n", i + 1, with_who(p)))
        .collect()
}

/// The to-dos as a Markdown task list, each with its owner and due date when known.
pub fn todos_markdown(m: &Meeting) -> String {
    m.summary
        .todo
        .iter()
        .map(|t| {
            let mut extra = Vec::new();
            if let Some(owner) = t.owner.as_deref().filter(|o| !o.is_empty()) {
                extra.push(owner.to_string());
            }
            if let Some(due) = t.due.as_deref().filter(|d| !d.is_empty()) {
                extra.push(format!("due {due}"));
            }
            if extra.is_empty() {
                format!("- [ ] {}\n", t.task)
            } else {
                format!("- [ ] {} ({})\n", t.task, extra.join(", "))
            }
        })
        .collect()
}

fn section(out: &mut String, title: &str, body: &str) {
    if body.trim().is_empty() {
        return;
    }
    out.push_str(&format!("\n## {title}\n\n{body}"));
}

fn bullets<'a>(items: impl IntoIterator<Item = &'a str>) -> String {
    items.into_iter().map(|t| format!("- {t}\n")).collect()
}

/// The AI notes: title, a facts line, then each section that has something in it.
pub fn notes_markdown(m: &Meeting) -> String {
    let mut out = format!("# {}\n", m.title);
    let mut facts = Vec::new();
    if let Some(at) = m.started {
        facts.push(at.format("%a %-d %b %Y, %H:%M").to_string());
    }
    if let Some(n) = m.minutes {
        facts.push(format!("{n} min"));
    }
    if !m.summary.kind.is_empty() {
        facts.push(m.summary.kind.clone());
    }
    if !m.summary.people.is_empty() {
        facts.push(m.summary.people.join(", "));
    }
    if !facts.is_empty() {
        out.push_str(&format!("\n{}\n", facts.join(" \u{b7} ")));
    }
    section(
        &mut out,
        "Context",
        &bullets(m.summary.context.iter().map(String::as_str)),
    );
    section(&mut out, "Decisions", &decisions_markdown(m));
    section(&mut out, "To do", &todos_markdown(m));
    let open: String = m
        .summary
        .to_settle
        .iter()
        .map(|q| match q.waits_on.as_deref().filter(|w| !w.is_empty()) {
            Some(w) => format!("- {} (waits on {w})\n", q.question),
            None => format!("- {}\n", q.question),
        })
        .collect();
    section(&mut out, "Still to settle", &open);
    if !m.summary.topics.is_empty() {
        out.push_str("\n## Discussion\n");
        for topic in &m.summary.topics {
            out.push_str(&format!("\n### {}\n\n", topic.title));
            for p in &topic.points {
                out.push_str(&format!("- {}\n", with_who(p)));
            }
        }
    }
    section(
        &mut out,
        "Check before acting",
        &bullets(m.summary.verify.iter().map(String::as_str)),
    );
    out
}

/// The notes plus the user's own notes, for the Export button. No transcript.
pub fn export_markdown(m: &Meeting) -> String {
    let mut out = notes_markdown(m);
    if !m.notes.trim().is_empty() {
        section(&mut out, "Your notes", &format!("{}\n", m.notes.trim_end()));
    }
    out
}

/// The transcript as plain lines: `[14:22] Maya: text`, with the parts a line lacks left out.
pub fn transcript_text(m: &Meeting) -> String {
    m.lines
        .iter()
        .map(|l| {
            let mut head = String::new();
            if !l.at.is_empty() {
                head.push_str(&format!("[{}] ", super::short(&l.at)));
            }
            if !l.who.is_empty() {
                head.push_str(&format!("{}: ", l.who));
            }
            format!("{head}{}\n", l.text)
        })
        .collect()
}

/// A file name for the export: the title in lower case letters and digits, then the meeting id.
/// Only ASCII letters, digits and dashes survive, so no title can name a path.
pub fn file_name(m: &Meeting) -> String {
    let mut slug = String::new();
    for c in m.title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.ends_with('-') && !slug.is_empty() {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_end_matches('-').chars().take(48).collect();
    let slug = slug.trim_end_matches('-');
    let id: String =
        m.id.chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
    match (slug.is_empty(), id.is_empty()) {
        (true, true) => "meeting.md".into(),
        (true, false) => format!("meeting-{id}.md"),
        (false, true) => format!("{slug}.md"),
        (false, false) => format!("{slug}-{id}.md"),
    }
}

/// Writes the export into `dir` (made if missing) and returns the file's path. Exporting the
/// same meeting again replaces its file.
pub fn export_to(dir: &Path, m: &Meeting) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;
    let path = dir.join(file_name(m));
    fs::write(&path, export_markdown(m))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::{Summary, Todo, Topic, Unsettled};

    fn meeting() -> Meeting {
        Meeting {
            folder: PathBuf::new(),
            device: None,
            id: "1800000000000-1".into(),
            title: "Northwind: onboarding review / launch".into(),
            summary: Summary {
                kind: "Planning".into(),
                people: vec!["Maya Chen".into(), "Priya Rao".into()],
                context: vec!["Northwind is changing its onboarding flow.".into()],
                topics: vec![Topic {
                    title: "Launch date".into(),
                    points: vec![
                        Point {
                            kind: "decided".into(),
                            text: "Ship in the second week of December.".into(),
                            who: Some("Maya Chen".into()),
                            at: vec![],
                        },
                        Point {
                            kind: "fact".into(),
                            text: "Pricing copy is late.".into(),
                            who: None,
                            at: vec![],
                        },
                    ],
                }],
                todo: vec![
                    Todo {
                        task: "Send the survey results".into(),
                        owner: Some("Maya Chen".into()),
                        due: Some("Friday".into()),
                        ..Default::default()
                    },
                    Todo {
                        task: "Draft the shorter step three".into(),
                        ..Default::default()
                    },
                ],
                to_settle: vec![Unsettled {
                    question: "Who owns the pricing page?".into(),
                    waits_on: Some("Priya Rao".into()),
                }],
                ..Default::default()
            },
            summary_raw: String::new(),
            transcript_raw: String::new(),
            notes: "Ask about the survey".into(),
            lines: Vec::new(),
            started: None,
            minutes: Some(42),
            export_path: None,
            search: String::new(),
        }
    }

    #[test]
    fn decisions_are_numbered_with_the_speaker() {
        assert_eq!(
            decisions_markdown(&meeting()),
            "1. Ship in the second week of December. (Maya Chen)\n"
        );
    }

    #[test]
    fn todos_carry_owner_and_due() {
        assert_eq!(
            todos_markdown(&meeting()),
            "- [ ] Send the survey results (Maya Chen, due Friday)\n- [ ] Draft the shorter step three\n"
        );
    }

    #[test]
    fn notes_leave_out_empty_sections_and_the_transcript() {
        let text = notes_markdown(&meeting());
        assert!(text.starts_with("# Northwind: onboarding review / launch\n"));
        assert!(text.contains("## Decisions\n\n1. Ship"));
        assert!(
            text.contains(
                "## Still to settle\n\n- Who owns the pricing page? (waits on Priya Rao)"
            )
        );
        assert!(text.contains("### Launch date"));
        assert!(!text.contains("Check before acting"));
        assert!(!text.contains("Your notes"));
    }

    #[test]
    fn export_adds_the_users_own_notes() {
        let text = export_markdown(&meeting());
        assert!(text.ends_with("## Your notes\n\nAsk about the survey\n"));
    }

    #[test]
    fn file_name_cannot_name_a_path() {
        let mut m = meeting();
        assert_eq!(
            file_name(&m),
            "northwind-onboarding-review-launch-1800000000000-1.md"
        );
        m.title = "..\\..\\Windows/System32".into();
        m.id = "..\\x".into();
        let name = file_name(&m);
        assert!(!name.contains(['/', '\\', ':']) && !name.starts_with('.'));
        m.title = "\u{4f1a}\u{8b70}".into();
        m.id = String::new();
        assert_eq!(file_name(&m), "meeting.md");
    }
}

//! Display-only titles. The original task remains the identity and full description.
pub fn split_title(task: &str) -> (&str, &str) {
    let text = task.trim();
    let dangling = text.split_whitespace().next_back().is_some_and(cannot_end);
    if text.chars().count() <= 45 && text.split_whitespace().count() <= 6 && !dangling {
        return (text, "");
    }
    // Match ASCII clause markers without changing the original UTF-8 offsets.
    let clause = text
        .char_indices()
        .find_map(|(i, _)| {
            let tail = &text[i..];
            [
                " so that ",
                " because ",
                " to ensure ",
                "(",
                ":",
                ";",
                ", ",
                " and ",
            ]
            .iter()
            .any(|stop| {
                tail.get(..stop.len())
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case(stop))
            })
            .then_some(i)
            .filter(|i| !text[..*i].trim().is_empty())
        })
        .unwrap_or(text.len());
    let mut end = 0;
    for (n, word) in text[..clause].split_whitespace().enumerate() {
        let start = text[end..clause].find(word).unwrap() + end;
        let next = start + word.len();
        if n == 6 || text[..next].chars().count() > 45 {
            break;
        }
        end = next;
    }
    // A long first word still gets a bounded, UTF-8-safe title.
    if end == 0 {
        end = text.char_indices().nth(45).map_or(text.len(), |(i, _)| i);
    }
    // A budget cut can leave only part of a prepositional phrase. Prefer its start
    // unless the next word opens a new phrase, leaving the selected one complete.
    if end < clause {
        let next = text[end..clause].split_whitespace().next();
        if !next.is_some_and(|word| {
            preposition(word)
                || ["and", "or", "but", "so"]
                    .iter()
                    .any(|stop| word.eq_ignore_ascii_case(stop))
        }) {
            let mut offset = 0;
            let mut phrase = None;
            for word in text[..end].split_whitespace() {
                let start = text[offset..end].find(word).unwrap() + offset;
                if start > 0 && preposition(word) {
                    phrase = Some(start);
                }
                offset = start + word.len();
            }
            if let Some(start) = phrase {
                end = start;
            }
        }
    }
    let mut title = text[..end].trim_end_matches([' ', ',', ':', ';', '.']);
    while let Some(last) = title.split_whitespace().next_back() {
        if !cannot_end(last) {
            break;
        }
        title = title[..title.len() - last.len()].trim_end();
    }
    (title, task)
}

fn preposition(word: &str) -> bool {
    [
        "in", "on", "at", "to", "for", "with", "from", "by", "of", "into", "before", "after",
    ]
    .iter()
    .any(|stop| word.eq_ignore_ascii_case(stop))
}

fn cannot_end(word: &str) -> bool {
    let word = word.trim_end_matches([',', ':', ';', '.']);
    preposition(word)
        || [
            "a", "an", "the", "all", "every", "each", "one", "some", "any", "this", "that", "and",
            "or", "but", "so",
        ]
        .iter()
        .any(|stop| word.eq_ignore_ascii_case(stop))
        || (!word.is_empty() && word.chars().all(char::is_numeric))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anonymised_meeting_shapes() {
        for (task, title) in [
            (
                "Put all production files in one tracker folder on the shared drive: production MIS, batch details and responsibilities",
                "Put all production files",
            ),
            (
                "Send the revised supplier tracker so that the operations team can confirm the next batch",
                "Send the revised supplier tracker",
            ),
            (
                "Review the production plan: confirm batch details, staffing and responsibilities with operations",
                "Review the production plan",
            ),
            (
                "Prepare the weekly report and share the batch details with the planning team before Friday",
                "Prepare the weekly report",
            ),
            (
                "Collect the current figures from the leads to ensure the monthly report reflects the latest results",
                "Collect the current figures",
            ),
            (
                "Share the updated production figures with the team because the current report is incomplete",
                "Share the updated production figures",
            ),
            (
                "Review the supplier schedule, confirm deliveries with operations and publish the updated tracker",
                "Review the supplier schedule",
            ),
        ] {
            let (short, full) = split_title(task);
            assert_eq!(short, title);
            assert_eq!(full, task);
            assert!(short.chars().count() <= 45);
            assert!(short.split_whitespace().count() <= 6);
        }
    }
    #[test]
    fn cuts_before_unfinished_prepositional_phrases() {
        for (task, expected) in [
            (
                "Put all production files in one tracker folder",
                "Put all production files",
            ),
            (
                "Send the revised figures to regional operations leads",
                "Send the revised figures",
            ),
            (
                "Share the latest batch details with the production team",
                "Share the latest batch details",
            ),
            (
                "Collect signed delivery receipts from each warehouse supervisor",
                "Collect signed delivery receipts",
            ),
            (
                "Move approved sample packs into the shared storage room",
                "Move approved sample packs",
            ),
        ] {
            assert_eq!(split_title(task), (expected, task));
        }
        // The next preposition starts a separate phrase; the preceding one is complete.
        assert_eq!(
            split_title("Send the report to the team before Friday morning").0,
            "Send the report to the team"
        );
    }

    #[test]
    fn removes_all_dangling_endings_repeatedly() {
        for word in [
            "in", "on", "at", "to", "for", "with", "from", "by", "of", "into", "before", "after",
            "a", "an", "the", "all", "every", "each", "one", "some", "any", "this", "that", "and",
            "or", "but", "so", "42", "123",
        ] {
            assert_eq!(
                split_title(&format!("Review the draft {word}")).0,
                "Review the draft"
            );
        }
        assert_eq!(
            split_title("Review the draft in one 42").0,
            "Review the draft"
        );
    }

    #[test]
    fn short_unicode_and_dangling_words() {
        assert_eq!(split_title("Send the report"), ("Send the report", ""));
        assert_eq!(split_title(""), ("", ""));
        let task = "Send all revised reports today directly to the operations team for their monthly review";
        assert_eq!(
            split_title(task).0,
            "Send all revised reports today directly"
        );
        assert_eq!(
            split_title("Send these files to the team and"),
            (
                "Send these files to the team",
                "Send these files to the team and"
            )
        );
        assert_eq!(
            split_title("Share all reports with the and the updated delivery team").0,
            "Share all reports"
        );
        assert_eq!(split_title("Finish the draft and").0, "Finish the draft");
        let long = "\u{754c}".repeat(90);
        assert_eq!(split_title(&long).0.chars().count(), 45);
        let unicode = "\u{130}nclude the caf\u{e9} report: share the revised production details with the operations team";
        assert_eq!(split_title(unicode).0, "\u{130}nclude the caf\u{e9} report");
    }
}

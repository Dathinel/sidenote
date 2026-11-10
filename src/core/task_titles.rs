//! Display-only titles. The original task remains the identity and full description.
pub fn split_title(task: &str) -> (&str, &str) {
    let text = task.trim();
    let dangling = text.split_whitespace().next_back().is_some_and(|word| {
        ["the", "to", "of", "and"].iter().any(|stop| {
            word.trim_end_matches([',', ':', ';', '.'])
                .eq_ignore_ascii_case(stop)
        })
    });
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
    let mut title = text[..end].trim_end_matches([' ', ',', ':', ';', '.']);
    while let Some((head, last)) = title.rsplit_once(char::is_whitespace) {
        if !["the", "to", "of", "and"]
            .iter()
            .any(|w| last.eq_ignore_ascii_case(w))
        {
            break;
        }
        title = head.trim_end();
    }
    (title, task)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn anonymised_meeting_shapes() {
        for (task, title) in [
            (
                "Put all production files in one tracker folder on the shared drive: production MIS, batch details and responsibilities",
                "Put all production files in one",
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
                "Collect the current figures from",
            ),
            (
                "Share the updated production figures with the team because the current report is incomplete",
                "Share the updated production figures with",
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
            "Share all reports with"
        );
        assert_eq!(split_title("Finish the draft and").0, "Finish the draft");
        let long = "\u{754c}".repeat(90);
        assert_eq!(split_title(&long).0.chars().count(), 45);
        let unicode = "\u{130}nclude the caf\u{e9} report: share the revised production details with the operations team";
        assert_eq!(split_title(unicode).0, "\u{130}nclude the caf\u{e9} report");
    }
}

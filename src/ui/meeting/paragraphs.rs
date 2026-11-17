//! Speech comes in short fragments ("what data", "okay", "you need there"). The transcript shows
//! a run from one speaker as one paragraph, under one time and one name.
use sidenote::core::Line;

/// A fragment more than this many seconds after the one before starts a new paragraph.
const MAX_GAP_SECONDS: u64 = 30;
/// A paragraph stops growing near this many characters, so a long monologue still has breaks.
const MAX_CHARS: usize = 600;

pub struct Paragraph<'a> {
    /// The first and last transcript line in the paragraph.
    pub first: usize,
    pub last: usize,
    /// The first line: its time, speaker and language stand for the paragraph.
    pub line: &'a Line,
    pub text: String,
}

/// Groups consecutive lines of one speaker and language. `apart` is a line that must stand on its
/// own, such as the one a summary point cites: it is found by its own key and highlight.
pub fn paragraphs(lines: &[Line], apart: Option<usize>) -> Vec<Paragraph<'_>> {
    let mut out: Vec<Paragraph> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if let Some(open) = out.last_mut()
            && !line.who.is_empty()
            && line.who == open.line.who
            && line.language == open.line.language
            && apart != Some(i)
            && apart != Some(open.last)
            && open.text.chars().count() + line.text.chars().count() < MAX_CHARS
            && match (lines[open.last].seconds, line.seconds) {
                (Some(before), Some(now)) => now.saturating_sub(before) <= MAX_GAP_SECONDS,
                _ => true,
            }
        {
            open.text.push(' ');
            open.text.push_str(line.text.trim());
            open.last = i;
            continue;
        }
        out.push(Paragraph {
            first: i,
            last: i,
            line,
            text: line.text.trim().to_string(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(seconds: u64, who: &str, text: &str) -> Line {
        Line {
            at: format!("{}:{:02}", seconds / 60, seconds % 60),
            seconds: Some(seconds),
            who: who.into(),
            language: "English".into(),
            text: text.into(),
        }
    }

    #[test]
    fn fragments_from_one_speaker_read_as_one_paragraph() {
        let lines = [
            line(0, "You", "Okay"),
            line(3, "You", "what data"),
            line(5, "You", "okay"),
            line(21, "Maya", "Sure"),
        ];
        let out = paragraphs(&lines, None);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].first, out[0].last), (0, 2));
        assert_eq!(out[0].text, "Okay what data okay");
        assert_eq!(out[0].line.at, "0:00");
        assert_eq!(
            (out[1].first, out[1].last, out[1].text.as_str()),
            (3, 3, "Sure")
        );
    }

    #[test]
    fn a_long_silence_or_a_new_language_starts_a_new_paragraph() {
        let mut lines = [
            line(0, "You", "Okay"),
            line(80, "You", "Good"),
            line(82, "You", "Fine"),
        ];
        assert_eq!(paragraphs(&lines, None).len(), 2);
        lines[2].language = "Telugu".into();
        assert_eq!(paragraphs(&lines, None).len(), 3);
    }

    #[test]
    fn lines_without_a_speaker_stay_separate() {
        let lines = [line(0, "", "First line"), line(2, "", "Second line")];
        assert_eq!(paragraphs(&lines, None).len(), 2);
    }

    #[test]
    fn the_cited_line_stays_on_its_own_so_it_can_be_found_and_marked() {
        let lines = [
            line(0, "You", "one"),
            line(2, "You", "two"),
            line(4, "You", "three"),
        ];
        let out = paragraphs(&lines, Some(1));
        let spans: Vec<_> = out.iter().map(|p| (p.first, p.last)).collect();
        assert_eq!(spans, [(0, 0), (1, 1), (2, 2)]);
    }

    #[test]
    fn a_monologue_still_breaks_into_paragraphs() {
        let words = "word ".repeat(40);
        let lines: Vec<Line> = (0..10).map(|i| line(i * 5, "Maya", &words)).collect();
        let out = paragraphs(&lines, None);
        assert!(out.len() > 1);
        assert!(out.iter().all(|p| p.text.chars().count() < MAX_CHARS + 210));
        assert_eq!(out.iter().map(|p| p.last - p.first + 1).sum::<usize>(), 10);
    }
}

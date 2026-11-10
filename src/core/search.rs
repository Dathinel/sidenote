use super::Meeting;
use std::ops::Range;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snippet {
    pub text: String,
    /// UTF-8 byte ranges into `text`, on character boundaries.
    pub matches: Vec<Range<usize>>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchSource {
    Title,
    Summary,
    Notes,
    Transcript,
}
#[derive(Clone, Debug)]
pub struct SearchHit {
    pub meeting_id: String,
    pub source: SearchSource,
    pub snippet: Snippet,
}
/// Case-insensitive matching with ranges in the original UTF-8 string.
pub fn match_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    let mut folded = String::new();
    let mut originals = Vec::new();
    for (at, ch) in text.char_indices() {
        for lower in ch.to_lowercase() {
            folded.push(lower);
            originals.extend(std::iter::repeat_n(
                at..at + ch.len_utf8(),
                lower.len_utf8(),
            ));
        }
    }
    let mut ranges = Vec::new();
    for (at, found) in folded.match_indices(&query) {
        let range = originals[at].start..originals[at + found.len() - 1].end;
        if ranges.last() != Some(&range) {
            ranges.push(range);
        }
    }
    ranges
}
pub fn make_snippet(text: &str, query: &str) -> Snippet {
    let ranges = match_ranges(text, query);
    let Some(first) = ranges.first() else {
        return Snippet {
            text: text.chars().take(140).collect(),
            matches: Vec::new(),
        };
    };
    let start = text[..first.start]
        .char_indices()
        .rev()
        .nth(34)
        .map_or(0, |(i, _)| i);
    let end = text[first.end..]
        .char_indices()
        .nth(85)
        .map_or(text.len(), |(i, _)| first.end + i);
    let prefix = if start > 0 { "..." } else { "" };
    let snippet = format!(
        "{prefix}{}{}",
        &text[start..end],
        if end < text.len() { "..." } else { "" }
    );
    let matches = ranges
        .into_iter()
        .filter(|r| r.start >= start && r.end <= end)
        .map(|r| r.start - start + prefix.len()..r.end - start + prefix.len())
        .collect();
    Snippet {
        text: snippet,
        matches,
    }
}
impl Meeting {
    pub fn search_hit(&self, query: &str) -> Option<SearchHit> {
        let q = query.trim().to_lowercase();
        if !self.search.contains(&q) {
            return None;
        }
        let found = std::iter::once((self.title.as_str(), SearchSource::Title))
            .chain(
                self.summary
                    .texts()
                    .map(|text| (text, SearchSource::Summary)),
            )
            .chain(std::iter::once((self.notes.as_str(), SearchSource::Notes)))
            .chain(
                self.lines
                    .iter()
                    .map(|l| (l.text.as_str(), SearchSource::Transcript)),
            )
            .chain(std::iter::once((
                self.transcript_raw.as_str(),
                SearchSource::Transcript,
            )))
            .find(|(text, _)| text.to_lowercase().contains(&q))?;
        Some(SearchHit {
            meeting_id: self.id.clone(),
            source: found.1,
            snippet: make_snippet(found.0, query),
        })
    }
}
pub fn search(meetings: &[Meeting], query: &str) -> Vec<SearchHit> {
    meetings
        .iter()
        .filter_map(|m| m.search_hit(query))
        .collect()
}

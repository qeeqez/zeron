//! `FindOpts` — the find bar's Match Case / Whole Word toggles and the
//! flag-aware matching they drive. `FindOpts::default()` reproduces the
//! pre-toggle behavior: a case-insensitive substring match.

use std::borrow::Cow;
use std::ops::Range;

use crate::model::ChatMessage;

/// How a find needle matches message text — the two chips beside the
/// query input. Both flags off is the classic case-insensitive substring
/// match; each flag narrows the hit set from there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct FindOpts {
    /// Compare case-exactly instead of folding both sides lowercase.
    pub case_sensitive: bool,
    /// Reject hits flanked by word chars (alphanumeric or `_`) — `hit`
    /// matches "(hit)" but not "hitter" or "a_hit".
    pub whole_word: bool,
}

impl FindOpts {
    /// Fold `s` for comparison — identity when case-sensitive, lowercase
    /// otherwise (the same fold `msg_matches` applies).
    fn fold<'a>(self, s: &'a str) -> Cow<'a, str> {
        if self.case_sensitive { Cow::Borrowed(s) } else { Cow::Owned(s.to_lowercase()) }
    }

    /// Whether `hay` contains `needle` under the flags. Whole-word scans
    /// every occurrence: a hit counts only when the chars flanking it —
    /// or the text's edges — aren't word chars.
    pub(crate) fn text_matches(self, hay: &str, needle: &str) -> bool {
        let hay = self.fold(hay);
        let needle = self.fold(needle);
        if !self.whole_word {
            return hay.contains(needle.as_ref());
        }
        hay.match_indices(needle.as_ref()).any(|(start, hit)| {
            let end = start + hit.len();
            !hay[..start].chars().next_back().is_some_and(word_char) && !hay[end..].chars().next().is_some_and(word_char)
        })
    }

    /// `chat_search::msg_matches` under the flags — a message matches when
    /// any of its haystacks (text, tool fields, plan labels…) does.
    pub(crate) fn msg_matches(self, m: &ChatMessage, needle: &str) -> bool {
        crate::chat_search::haystacks(m).iter().any(|h| self.text_matches(h, needle))
    }

    /// Byte range of `needle`'s first hit in `hay` under the flags — `None`
    /// on a miss. Comparison is char-wise (each side's lowercase form), so
    /// the range slices `hay` directly even when a fold changes byte counts
    /// (İ → i̇): the spans a highlight or snippet window paints are always
    /// the text that actually matched. `text_matches`' whole-string fold
    /// may differ on multi-char expansions — spans stay self-consistent.
    pub(crate) fn first_match(self, hay: &str, needle: &str) -> Option<Range<usize>> {
        self.find_at(hay, needle, 0)
    }

    /// Every non-overlapping `needle` hit in `hay` — `find_at` stepped past
    /// each span, so hits never share bytes. An empty needle matches
    /// nothing (and would never advance the scan).
    pub(crate) fn match_ranges(self, hay: &str, needle: &str) -> Vec<Range<usize>> {
        let mut ranges = Vec::new();
        let mut from = 0;
        while !needle.is_empty() {
            let Some(r) = self.find_at(hay, needle, from) else { break };
            from = r.end;
            ranges.push(r);
        }
        ranges
    }

    /// `first_match` at or after byte offset `from`.
    fn find_at(self, hay: &str, needle: &str, from: usize) -> Option<Range<usize>> {
        hay.char_indices().skip_while(|(ix, _)| *ix < from).find_map(|(ix, _)| {
            let range = ix..ix + self.match_prefix(&hay[ix..], needle)?;
            (!self.whole_word || word_bounded(hay, &range)).then_some(range)
        })
    }

    /// Bytes of `hay`'s leading run that equal `needle` under the flags —
    /// `None` on a miss. Char-wise so the returned length indexes `hay`
    /// itself; a multi-char lowercase (e.g. İ → i̇) still lines up.
    fn match_prefix(self, hay: &str, needle: &str) -> Option<usize> {
        let mut len = 0;
        let mut hs = hay.chars();
        for q in needle.chars() {
            match hs.next() {
                Some(h) if self.char_eq(h, q) => len += h.len_utf8(),
                _ => return None,
            }
        }
        Some(len)
    }

    /// Char compare under the flags — exact when case-sensitive, each
    /// char's lowercase form otherwise.
    fn char_eq(self, h: char, q: char) -> bool {
        if self.case_sensitive { h == q } else { h.to_lowercase().eq(q.to_lowercase()) }
    }
}

/// Both flanks of `range` in `hay` are non-word chars or `hay`'s edges —
/// the whole-word rule applied to one candidate hit.
fn word_bounded(hay: &str, range: &Range<usize>) -> bool {
    !hay[..range.start].chars().next_back().is_some_and(word_char) && !hay[range.end..].chars().next().is_some_and(word_char)
}

/// Word chars for whole-word boundaries — the `\w` rule: letters, digits
/// and `_`, unicode-aware so `café` is one word but `snake_case` splits.
fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

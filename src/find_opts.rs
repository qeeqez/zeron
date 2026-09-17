//! `FindOpts` — the find bar's Match Case / Whole Word toggles and the
//! flag-aware matching they drive. `FindOpts::default()` reproduces the
//! pre-toggle behavior: a case-insensitive substring match.

use std::borrow::Cow;

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
}

/// Word chars for whole-word boundaries — the `\w` rule: letters, digits
/// and `_`, unicode-aware so `café` is one word but `snake_case` splits.
fn word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

//! Fuzzy subsequence scoring for the command palette — the same shape as
//! fzf/VS Code: every query char must appear in order; tighter, earlier,
//! boundary-aligned matches score higher.

/// Score `candidate` against `query`. `None` when the query isn't a
/// subsequence of the candidate (case-insensitive). Higher is better.
pub(crate) fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    let query = query.trim();
    if query.is_empty() {
        return Some(0);
    }
    let qchars: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let chars: Vec<char> = candidate.chars().collect();
    let mut score = 0i32;
    let mut qi = 0usize;
    let mut prev_match: Option<usize> = None;
    for (i, ch) in chars.iter().enumerate() {
        if qi >= qchars.len() {
            break;
        }
        if !ch.to_lowercase().any(|c| c == qchars[qi]) {
            continue;
        }
        score += 10;
        if prev_match.is_some_and(|p| i == p + 1) {
            score += 8; // consecutive run
        }
        if i == 0 || is_boundary(chars[i - 1], *ch) {
            score += 6; // start of candidate or a word
        }
        if let Some(p) = prev_match {
            score -= (i - p - 1) as i32; // gap penalty
        }
        prev_match = Some(i);
        qi += 1;
    }
    (qi == qchars.len()).then_some(score)
}

/// Word-boundary heuristic: after whitespace/punctuation, or a camelCase hump
/// (lower/digit → upper).
fn is_boundary(prev: char, cur: char) -> bool {
    !prev.is_alphanumeric() || (cur.is_uppercase() && (prev.is_lowercase() || prev.is_ascii_digit()))
}

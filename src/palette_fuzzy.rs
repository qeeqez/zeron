//! Fuzzy subsequence scoring for the command palette — the same shape as
//! fzf/VS Code: every query char must appear in order; tighter, earlier,
//! boundary-aligned matches score higher.

/// Sentinel for "no alignment reaches this cell" — kept far above `i32::MIN`
/// so score arithmetic on it can't wrap into a plausible value.
const NONE: i32 = i32::MIN / 2;

/// Score `candidate` against `query`. `None` when the query isn't a
/// subsequence of the candidate (case-insensitive). Higher is better.
///
/// Scores the *best* subsequence alignment, not the first: `prev[q]` is the
/// top score for matching the first `q` query chars with the q-th landing on
/// the previous candidate char, so a late contiguous run beats an early
/// scattered one.
pub(crate) fn fuzzy_score(query: &str, candidate: &str) -> Option<i32> {
    let query = query.trim();
    if query.is_empty() {
        return Some(0);
    }
    let qchars: Vec<char> = query.chars().flat_map(char::to_lowercase).collect();
    let chars: Vec<char> = candidate.chars().collect();
    let mut prev = vec![NONE; chars.len()];
    let mut cur = vec![NONE; chars.len()];
    for (qi, &q) in qchars.iter().enumerate() {
        // `best` = max over j < i of `prev[j] - (i - j - 1)` = `prev[j] + j - i + 1`,
        // i.e. the best earlier alignment once the gap penalty is applied.
        let mut best = NONE;
        for (i, &ch) in chars.iter().enumerate() {
            if i > 0 && prev[i - 1] != NONE {
                best = best.max(prev[i - 1].saturating_add(i as i32 - 1));
            }
            cur[i] = NONE;
            if !ch.to_lowercase().any(|c| c == q) {
                continue;
            }
            let mut score = 10;
            if i == 0 || is_boundary(chars[i - 1], ch) {
                score += 6; // start of candidate or a word
            }
            if qi == 0 {
                cur[i] = score;
                continue;
            }
            // Gap match via `best`, or extend the run at i-1 for the
            // consecutive bonus (a zero-gap match without it).
            let via_gap = (best != NONE).then(|| best + 1 - i as i32);
            let via_run = (i > 0 && prev[i - 1] != NONE).then(|| prev[i - 1] + 8);
            if let Some(base) = via_gap.into_iter().chain(via_run).max() {
                cur[i] = score + base;
            }
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    let best = prev.iter().max().copied().unwrap_or(NONE);
    (best != NONE).then_some(best)
}

/// Word-boundary heuristic: after whitespace/punctuation, or a camelCase hump
/// (lower/digit → upper).
fn is_boundary(prev: char, cur: char) -> bool {
    !prev.is_alphanumeric() || (cur.is_uppercase() && (prev.is_lowercase() || prev.is_ascii_digit()))
}

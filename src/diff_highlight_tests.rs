//! Tests for `diff_highlight` — `changed_spans` prefix/suffix trimming and
//! `line_spans` removed↔added pairing. Sibling file so `diff_highlight.rs`
//! stays under the SLOC cap.

#[cfg(test)]
mod tests {
    use crate::changes_diff::diff_highlight::{changed_spans, line_spans};
    use crate::changes_diff::parse_diff;

    #[test]
    fn identical_lines_mark_nothing() {
        assert_eq!(changed_spans("same line", "same line"), (vec![], vec![]));
        assert_eq!(changed_spans("", ""), (vec![], vec![]));
    }

    #[test]
    fn prefix_change_marks_the_head() {
        let (old, new) = changed_spans("foo bar", "baz bar");
        assert_eq!(old, vec![(0, 3)]);
        assert_eq!(new, vec![(0, 3)]);
    }

    #[test]
    fn suffix_change_marks_the_tail() {
        let (old, new) = changed_spans("let x = 1;", "let x = 42;");
        assert_eq!(old, vec![(8, 9)]);
        assert_eq!(new, vec![(8, 10)]);
    }

    #[test]
    fn middle_change_keeps_both_ends() {
        let (old, new) = changed_spans("call(foo, bar)", "call(baz, bar)");
        assert_eq!(old, vec![(5, 8)]);
        assert_eq!(new, vec![(5, 8)]);
    }

    #[test]
    fn completely_different_lines_mark_everything() {
        let (old, new) = changed_spans("abc", "wxyz");
        assert_eq!(old, vec![(0, 3)]);
        assert_eq!(new, vec![(0, 4)]);
    }

    #[test]
    fn empty_sides_mark_the_other_line() {
        // An empty old line is all-change on the new side; the old span is an
        // empty range at offset 0 — nothing to paint, but still a boundary.
        let (old, new) = changed_spans("", "added");
        assert_eq!(old, vec![(0, 0)]);
        assert_eq!(new, vec![(0, 5)]);
        let (old, new) = changed_spans("gone", "");
        assert_eq!(old, vec![(0, 4)]);
        assert_eq!(new, vec![(0, 0)]);
    }

    #[test]
    fn insertion_marks_only_the_inserted_bytes() {
        // "world" inserted into "hello " — the old side's span is the empty
        // range at the insertion point, the new side's is the inserted text.
        let (old, new) = changed_spans("hello ", "hello world");
        assert_eq!(old, vec![(6, 6)]);
        assert_eq!(new, vec![(6, 11)]);
    }

    #[test]
    fn spans_stay_on_char_boundaries() {
        // 'ö' is two bytes — a byte-naive trim would split it and hand
        // `StyledText` a non-boundary range.
        let (old, new) = changed_spans("héllo wörld", "héllo world");
        assert_eq!(old, vec![(8, 10)], "'ö' is bytes 8..10");
        assert_eq!(new, vec![(8, 9)]);
    }

    #[test]
    fn line_spans_pairs_removed_with_added_indexwise() {
        // -a -b | +x +y +z — the first two pair off; the third added line is
        // unpaired and carries no mark.
        let diff = parse_diff("@@ -1,2 +1,3 @@\n-foo a\n-foo b\n+foo x\n+foo y\n+foo z\n");
        let marks = line_spans(&diff);
        assert_eq!(marks[1], vec![(4, 5)], "first removed pairs with first added");
        assert_eq!(marks[2], vec![(4, 5)]);
        assert_eq!(marks[3], vec![(4, 5)]);
        assert_eq!(marks[4], vec![(4, 5)]);
        assert!(marks[5].is_empty(), "unpaired added line gets no mark");
    }

    #[test]
    fn line_spans_leaves_context_and_hunks_unmarked() {
        let diff = parse_diff("@@ -1,3 +1,3 @@\n keep\n-old\n+new\n tail\n");
        let marks = line_spans(&diff);
        assert!(marks[0].is_empty(), "hunk header");
        assert!(marks[1].is_empty(), "context line");
        assert_eq!(marks[2], vec![(0, 3)]);
        assert_eq!(marks[3], vec![(0, 3)]);
        assert!(marks[4].is_empty(), "context line");
    }

    #[test]
    fn line_spans_skips_identical_pairs() {
        // A removed/added pair whose text is equal (whitespace-only edits
        // under --ignore-all-space never reach here, but a same-text pair
        // still can) marks nothing on either side.
        let diff = parse_diff("@@ -1,1 +1,1 @@\n-same\n+same\n");
        let marks = line_spans(&diff);
        assert!(marks[1].is_empty());
        assert!(marks[2].is_empty());
    }
}

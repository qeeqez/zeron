//! Command blocks for the embedded terminal: each line the input row
//! submits becomes a block — the command plus the output it produced —
//! so the panel can render Codex-style sections with per-block actions
//! instead of one flat text dump.
//!
//! Boundaries are tracked in "eternal" row coordinates: `pushed` counts
//! every row that ever scrolled off the screen top, so `pushed + row`
//! names a transcript row that stays fixed as scrollback grows (the
//! transcript index shifts only when our own history cap drops rows).
//! When the shell emits OSC 133 marks (`unhandled_osc` sees them) the
//! output start and exit code are exact; otherwise the output start is
//! estimated from the echo height — prompt text is never parsed.

use std::collections::VecDeque;

use super::{SCROLLBACK, TermSession};

/// Rows of transcript history kept per session — twice the parser's
/// scrollback so block boundaries stay exact after vt100 starts dropping.
const HISTORY_CAP: usize = SCROLLBACK * 2;

/// One grid row of transcript text. `wrapped` means the row continues
/// into the next (a soft wrap, not a newline) — the same flag vt100 sets.
#[derive(Clone, Debug)]
pub(crate) struct Row {
    pub text: String,
    pub wrapped: bool,
}

/// A submitted command and its region in the transcript. `start` is the
/// row the shell echoes the command onto (the header replaces it);
/// `output` is where output begins; `end` is set only by an OSC 133;D
/// mark — otherwise the next block's `start` bounds this block.
#[derive(Clone, Debug)]
pub(crate) struct CmdBlock {
    pub command: String,
    pub start: u64,
    pub output: u64,
    pub end: Option<u64>,
    pub exit: Option<i64>,
    /// An OSC 133;C mark already pinned `output` — later stray marks
    /// (a bare Enter's cycle lands on this block) must not move it.
    osc_c: bool,
    /// Same latch for 133;D — the first `end`/`exit` wins.
    osc_d: bool,
}

impl CmdBlock {
    /// A fresh block: `output` starts as the echo-height estimate until
    /// an OSC 133;C mark pins it.
    pub(crate) fn new(command: String, start: u64, output: u64) -> Self {
        Self {
            command,
            start,
            output,
            end: None,
            exit: None,
            osc_c: false,
            osc_d: false,
        }
    }
}

/// A shell-integration mark captured mid-parse: the OSC 133 kind plus
/// the raw transcript position (`scrollback rows + cursor row`) at the
/// moment it arrived. `drain` converts it once scrollback is synced.
#[derive(Debug)]
enum Mark {
    PromptStart(u64),
    OutputStart(u64),
    Done(u64, Option<i64>),
}

/// vt100 callbacks: OSC 133 shell-integration marks are the only ones we
/// consume. They arrive while `process` runs, before `drain`'s scrollback
/// sync, so they're queued raw and applied afterwards.
#[derive(Default)]
pub(crate) struct TermCallbacks {
    marks: Vec<Mark>,
}

/// The current scrollback length, read by briefly scrolling the view to
/// the top — vt100 exposes no direct length getter. Restored at once.
fn scrollback_len(screen: &mut vt100::Screen) -> usize {
    screen.set_scrollback(usize::MAX);
    let len = screen.scrollback();
    screen.set_scrollback(0);
    len
}

fn atoi(bytes: &[u8]) -> Option<i64> {
    std::str::from_utf8(bytes).ok()?.trim().parse().ok()
}

impl vt100::Callbacks for TermCallbacks {
    fn unhandled_osc(&mut self, screen: &mut vt100::Screen, params: &[&[u8]]) {
        let [b"133", code, rest @ ..] = params else {
            return;
        };
        let raw = scrollback_len(screen) as u64 + u64::from(screen.cursor_position().0);
        let mark = match *code {
            b"A" => Mark::PromptStart(raw),
            b"C" => Mark::OutputStart(raw),
            b"D" => Mark::Done(raw, rest.first().and_then(|c| atoi(c))),
            _ => return,
        };
        self.marks.push(mark);
    }
}

/// The transcript as logical lines: scrollback history joined with the
/// visible screen, soft-wrapped rows merged. `starts[i]` is the
/// transcript row index where line `i` begins; `rows` counts every row
/// (including any trailing blank ones the line list trims).
pub(crate) struct Transcript {
    pub lines: Vec<String>,
    pub starts: Vec<u64>,
    pub rows: u64,
}

/// One block's placement over the transcript lines: `hide` covers the
/// echoed command (the header stands in for it) and `out` is the output
/// range — the header row is inserted where `hide` begins.
#[derive(Clone, Debug)]
pub(crate) struct BlockLayout {
    pub command: String,
    pub exit: Option<i64>,
    pub hide: std::ops::Range<usize>,
    pub out: std::ops::Range<usize>,
}

/// Rows `k` back from the scrollback tail, oldest first, paged through
/// the scrollback-offset window (`take` alone can't pass the screen
/// height). Only called for rows the parser still holds (`k <= len`).
fn read_sb_rows(screen: &mut vt100::Screen, k: usize) -> Vec<Row> {
    let (rows_len, cols) = screen.size();
    let rows_len = usize::from(rows_len);
    let mut out = Vec::with_capacity(k);
    while out.len() < k {
        let take = rows_len.min(k - out.len());
        screen.set_scrollback(k - out.len());
        out.extend(
            screen
                .rows(0, cols)
                .take(take)
                .enumerate()
                .map(|(i, text)| Row { text, wrapped: screen.row_wrapped(i as u16) }),
        );
    }
    screen.set_scrollback(0);
    out
}

/// How many rows `new_tail` was pushed by: the overlap between the old
/// tail's suffix and the new tail's prefix. Scrollback rows are immutable
/// once pushed, so a mismatch means more rows arrived than the tail
/// covers — report the whole tail as new.
fn tail_overlap(old: &VecDeque<String>, new: &VecDeque<String>) -> usize {
    let n = old.len().min(new.len());
    (0..n).find(|&k| old.iter().skip(k).eq(new.iter().take(n - k))).unwrap_or(n)
}

impl TermSession {
    /// The transcript: history rows followed by the live screen, merged
    /// into logical lines. Trailing blank lines are dropped so the text
    /// matches what `contents()` always returned.
    pub(crate) fn transcript(&self) -> Transcript {
        let screen = self.screen.screen();
        let (_, cols) = screen.size();
        let rows: Vec<Row> = self
            .history
            .iter()
            .cloned()
            .chain(
                screen
                    .rows(0, cols)
                    .enumerate()
                    .map(|(i, text)| Row { text, wrapped: screen.row_wrapped(i as u16) }),
            )
            .collect();
        let mut lines: Vec<String> = Vec::new();
        let mut starts: Vec<u64> = Vec::new();
        let mut line = String::new();
        let mut line_start = 0u64;
        let mut started = false;
        for (i, r) in rows.iter().enumerate() {
            if !started {
                line_start = i as u64;
                started = true;
            }
            line.push_str(&r.text);
            if !r.wrapped {
                starts.push(line_start);
                lines.push(std::mem::take(&mut line));
                started = false;
            }
        }
        if started {
            starts.push(line_start);
            lines.push(line);
        }
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
            starts.pop();
        }
        Transcript { lines, starts, rows: rows.len() as u64 }
    }

    /// Eternal coordinate → transcript row index. Rows dropped from the
    /// front of `history` shift every index down; marks before the
    /// transcript start clamp to 0.
    fn row_ix(&self, e: u64) -> u64 {
        e + self.history.len() as u64 - self.pushed
    }

    /// Lay the recorded blocks over the transcript: each block hides the
    /// lines its echo occupies and owns the output up to the next block's
    /// echo (or its OSC 133;D end / the last prompt start / the end).
    pub(crate) fn block_layout(&self, t: &Transcript) -> Vec<BlockLayout> {
        let mut out = Vec::with_capacity(self.blocks.len());
        for (i, b) in self.blocks.iter().enumerate() {
            let start = self.row_ix(b.start).min(t.rows);
            let output = self.row_ix(b.output).min(t.rows).max(start);
            let end = b
                .end
                .map(|e| self.row_ix(e))
                .or_else(|| self.blocks.get(i + 1).map(|n| self.row_ix(n.start)))
                .or(self.last_prompt.map(|e| self.row_ix(e)))
                .unwrap_or(t.rows)
                .min(t.rows)
                .max(output);
            let hide_start = t.starts.partition_point(|&s| s < start);
            let hide_end = t.starts.partition_point(|&s| s < output);
            let out_start = t.starts.partition_point(|&s| s < end).min(t.lines.len());
            out.push(BlockLayout {
                command: b.command.clone(),
                exit: b.exit,
                hide: hide_start..hide_end,
                out: hide_end..out_start.max(hide_end),
            });
        }
        out
    }

    /// The block's output text — the transcript lines between its echo
    /// and the next boundary, for the copy affordance.
    pub(crate) fn block_output(&self, ix: usize) -> String {
        let t = self.transcript();
        self.block_layout(&t).get(ix).map_or_else(String::new, |bl| t.lines[bl.out.clone()].join("\n"))
    }

    /// Mirror newly scrolled-off rows into `history` and keep `pushed`
    /// exact. Below the parser's cap the scrollback length grows per push;
    /// at the cap the immutable tail diff counts them instead.
    pub(crate) fn sync_scrollback(&mut self) {
        let screen = self.screen.screen_mut();
        let sb = scrollback_len(screen);
        if sb < self.sb_len {
            // Scrollback shrank (never observed, but don't mirror stale
            // rows): restart the transcript rather than drift.
            self.history.clear();
            self.sb_tail.clear();
            self.blocks.clear();
            self.pushed = 0;
            self.sb_len = sb;
            return;
        }
        let rows_len = usize::from(screen.size().0);
        let k = if sb > self.sb_len {
            sb - self.sb_len
        } else if sb == SCROLLBACK && !self.sb_tail.is_empty() {
            // At the parser's cap the length stops growing — diff the
            // immutable tail to count pushes instead.
            let n = rows_len.min(sb);
            screen.set_scrollback(n);
            let (_, cols) = screen.size();
            let new_tail: VecDeque<String> = screen.rows(0, cols).take(n).collect();
            screen.set_scrollback(0);
            tail_overlap(&self.sb_tail, &new_tail)
        } else {
            0
        };
        if k > 0 {
            let rows = read_sb_rows(screen, k.min(sb));
            for row in &rows {
                self.sb_tail.push_back(row.text.clone());
            }
            self.history.extend(rows);
            self.pushed += k as u64;
            while self.history.len() > HISTORY_CAP {
                self.history.pop_front();
            }
            while self.sb_tail.len() > rows_len {
                self.sb_tail.pop_front();
            }
            self.sb_len = sb;
        }
    }

    /// Apply the OSC 133 marks queued during `process`. `raw` was
    /// `scrollback + cursor row` at mark time; adding the rows the parser
    /// has since dropped (`pushed - scrollback len`) yields the eternal
    /// coordinate the blocks use.
    pub(crate) fn apply_marks(&mut self) {
        let marks = std::mem::take(&mut self.screen.callbacks_mut().marks);
        for mark in marks {
            self.apply_mark(mark);
        }
    }

    /// One queued mark: `raw` was `scrollback + cursor row` at mark
    /// time; adding the rows the parser has since dropped (`pushed -
    /// scrollback len`) yields the eternal coordinate the blocks use.
    fn apply_mark(&mut self, mark: Mark) {
        let e = match mark {
            Mark::PromptStart(raw) | Mark::OutputStart(raw) | Mark::Done(raw, _) => raw,
        } + (self.pushed - self.sb_len as u64);
        match mark {
            Mark::PromptStart(_) => self.last_prompt = Some(e),
            Mark::OutputStart(_) => {
                if let Some(b) = self.block_at_mut(e)
                    && !b.osc_c
                {
                    b.output = e;
                    b.osc_c = true;
                }
            },
            Mark::Done(_, code) => {
                if let Some(b) = self.block_at_mut(e)
                    && !b.osc_d
                {
                    b.end = Some(e);
                    b.exit = code;
                    b.osc_d = true;
                }
            },
        }
    }

    /// The block a mark belongs to: the last one submitted at or before
    /// the mark's position. Stray marks (bare Enter, a command submitted
    /// while another ran) land on the previous block and are ignored
    /// because its marks are already set.
    fn block_at_mut(&mut self, e: u64) -> Option<&mut CmdBlock> {
        self.blocks.iter_mut().rev().find(|b| b.start <= e)
    }
}

//! Git status decorations for the Files explorer — a read-only view over the
//! workspace's `changes` snapshot (the same `git status` collection the
//! Changes panel renders; nothing here shells out). File rows get a
//! right-aligned status letter, directory rows a dot when any descendant is
//! dirty.

use std::collections::HashMap;

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::git::{ChangeStatus, FileChange};

/// The severity bucket a badge paints with — resolved against the theme at
/// render time so the map stays theme-free and tests can assert the tone.
/// `Ord` is severity order: a dir aggregates to its worst descendant tone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Tone {
    Success,
    Info,
    Warning,
    Danger,
}

/// One file's explorer badge: the status letter plus its a11y label.
#[derive(Clone, Copy, Debug)]
pub(crate) struct GitBadge {
    pub letter: &'static str,
    pub label: &'static str,
    pub tone: Tone,
}

impl GitBadge {
    /// `Added` splits on `staged`: staged adds read `A`, untracked files `?`
    /// (porcelain reports both as adds — `staged` is the X column).
    fn of(change: &FileChange) -> Self {
        let (letter, label, tone) = match change.status {
            ChangeStatus::Added if change.staged => ("A", "added", Tone::Success),
            ChangeStatus::Added => ("?", "untracked", Tone::Info),
            ChangeStatus::Modified => ("M", "modified", Tone::Warning),
            ChangeStatus::Deleted => ("D", "deleted", Tone::Danger),
            ChangeStatus::Renamed => ("R", "renamed", Tone::Info),
            ChangeStatus::Conflicted => ("C", "conflicted", Tone::Danger),
        };
        Self { letter, label, tone }
    }
}

/// Per-render lookup built once from `Workspace::changes`: file badges by
/// project-relative path, plus every ancestor dir of a changed path marked
/// dirty with its worst descendant tone. Empty changes → empty maps → the
/// explorer renders exactly as it does outside a repo.
#[derive(Default)]
pub(crate) struct GitDecorations {
    files: HashMap<String, GitBadge>,
    dirs: HashMap<String, Tone>,
}

impl GitDecorations {
    pub(crate) fn build(changes: &[FileChange]) -> Self {
        let mut decorations = Self::default();
        for change in changes {
            let badge = GitBadge::of(change);
            decorations.files.insert(change.path.clone(), badge);
            // Every ancestor segment is dirty — "a/b/c.rs" marks "a/b" + "a".
            let mut dir = change.path.as_str();
            while let Some((parent, _)) = dir.rsplit_once('/') {
                let tone = decorations.dirs.entry(parent.to_string()).or_insert(badge.tone);
                *tone = (*tone).max(badge.tone);
                dir = parent;
            }
        }
        decorations
    }

    pub(crate) fn file(&self, path: &str) -> Option<GitBadge> {
        self.files.get(path).copied()
    }

    pub(crate) fn dir(&self, path: &str) -> Option<Tone> {
        self.dirs.get(path).copied()
    }
}

fn color(tone: Tone, cx: &App) -> Hsla {
    let theme = cx.theme();
    match tone {
        Tone::Success => theme.success,
        Tone::Info => theme.info,
        Tone::Warning => theme.warning,
        Tone::Danger => theme.danger,
    }
}

/// The right-aligned status letter on a file row.
pub(crate) fn badge_element(ix: usize, badge: GitBadge, cx: &App) -> AnyElement {
    div()
        .id(("explorer-badge", ix))
        .test_support()
        .aria_label(format!("git status: {}", badge.label))
        .flex_shrink_0()
        .text_xs()
        .text_color(color(badge.tone, cx))
        .child(badge.letter)
        .into_any_element()
}

/// The dot on a directory row with a dirty descendant.
pub(crate) fn dirty_dot(ix: usize, tone: Tone, cx: &App) -> AnyElement {
    div()
        .id(("explorer-dirty", ix))
        .test_support()
        .aria_label("contains changes")
        .flex_shrink_0()
        .size(px(6.))
        .rounded_full()
        .bg(color(tone, cx))
        .into_any_element()
}

use gpui_kit::{Context, SharedString};

use crate::workspace::Workspace;

/// Cap on candidates so the @-mention filter stays cheap on huge trees.
const MAX_FILES: usize = 2000;

/// Collect @-mention candidates relative to the project root.
/// Prefers `git ls-files` (respects .gitignore, includes untracked files);
/// falls back to a bounded directory walk outside git work trees.
pub fn scan_project_files(root: &std::path::Path) -> Vec<SharedString> {
    git_ls_files(root).unwrap_or_else(|| walk_files(root))
}

/// Tracked + untracked, non-ignored files, relative to `root`.
fn git_ls_files(root: &std::path::Path) -> Option<Vec<SharedString>> {
    let out = std::process::Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "-z"])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let mut files: Vec<SharedString> = out
        .stdout
        .split(|b| *b == 0)
        .filter(|p| !p.is_empty())
        .take(MAX_FILES)
        .map(|p| SharedString::from(String::from_utf8_lossy(p).into_owned()))
        .collect();
    files.sort();
    Some(files)
}

/// Files only, depth ≤ 4, skips build/VCS dirs.
fn walk_files(root: &std::path::Path) -> Vec<SharedString> {
    const SKIP: [&str; 6] = ["target", ".git", "node_modules", ".idea", ".sloc-guard", "dist"];
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if out.len() >= MAX_FILES {
            break;
        }
        if depth > 4 {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if SKIP.contains(&name.as_str()) {
                continue;
            }
            if path.is_dir() {
                stack.push((path, depth + 1));
            } else if let Ok(rel) = path.strip_prefix(root) {
                out.push(SharedString::from(rel.to_string_lossy().into_owned()));
            }
        }
    }
    out.sort();
    out
}

/// A directory in the explorer tree: nested `dirs` first (sorted), then
/// `files` (sorted full project-relative paths). `path` is the
/// project-relative dir path — "" for the root.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DirNode {
    pub name: SharedString,
    pub path: SharedString,
    pub dirs: Vec<DirNode>,
    pub files: Vec<SharedString>,
}

/// Group a flat project-relative file list (`scan_project_files`) into the
/// directory tree the explorer renders. Empty segments are skipped, so a
/// stray `a//b` or leading `/` can't create nameless dirs.
pub fn build_file_tree(files: &[SharedString]) -> DirNode {
    let mut root = DirNode::default();
    for file in files {
        let mut parts = file.split('/').filter(|s| !s.is_empty()).peekable();
        let mut node = &mut root;
        let mut dir_path = String::new();
        while let Some(part) = parts.next() {
            if parts.peek().is_none() {
                node.files.push(file.clone());
                break;
            }
            if !dir_path.is_empty() {
                dir_path.push('/');
            }
            dir_path.push_str(part);
            let ix = match node.dirs.iter().position(|d| d.name == part) {
                Some(ix) => ix,
                None => {
                    node.dirs.push(DirNode {
                        name: SharedString::from(part.to_string()),
                        path: SharedString::from(dir_path.clone()),
                        ..DirNode::default()
                    });
                    node.dirs.len() - 1
                },
            };
            node = &mut node.dirs[ix];
        }
    }
    sort_dirs(&mut root);
    root
}

fn sort_dirs(node: &mut DirNode) {
    node.dirs.sort_by(|a, b| a.name.cmp(&b.name));
    node.files.sort();
    for dir in &mut node.dirs {
        sort_dirs(dir);
    }
}

/// Explorer file ops (new/rename/delete) — declared here, not in `main.rs`,
/// which is at the SLOC cap (same pattern as `views/mod.rs`'s `#[path]`s).
#[path = "fs_ops.rs"]
pub(crate) mod fs_ops;

impl Workspace {
    /// Kick off the project-file scan for the @-mention picker on the
    /// background executor — a large tree would block launch, so the
    /// picker just stays empty until it lands. Called once from
    /// `lifecycle::start_background`.
    pub(crate) fn start_file_scan(&self, cx: &mut Context<Self>) {
        let root = self.project.root().to_path_buf();
        cx.spawn(async move |this, cx| {
            let files = cx.background_executor().spawn(async move { scan_project_files(&root) }).await;
            let _ = this.update(cx, |this, cx| {
                this.project_files = files;
                cx.notify();
            });
        })
        .detach();
    }
}

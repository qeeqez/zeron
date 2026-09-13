use gpui_kit::SharedString;

/// Cap on candidates so the @-mention filter stays cheap on huge trees.
const MAX_FILES: usize = 2000;

/// Collect @-mention candidates relative to the working directory.
/// Prefers `git ls-files` (respects .gitignore, includes untracked files);
/// falls back to a bounded directory walk outside git work trees.
pub fn scan_project_files() -> Vec<SharedString> {
    git_ls_files().unwrap_or_else(walk_files)
}

/// Tracked + untracked, non-ignored files, relative to the cwd.
fn git_ls_files() -> Option<Vec<SharedString>> {
    let out = std::process::Command::new("git")
        .args(["ls-files", "--cached", "--others", "--exclude-standard", "-z"])
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
fn walk_files() -> Vec<SharedString> {
    const SKIP: [&str; 6] = ["target", ".git", "node_modules", ".idea", ".sloc-guard", "dist"];
    let root = std::env::current_dir().unwrap_or_default();
    let mut out = Vec::new();
    let mut stack = vec![(root.clone(), 0usize)];
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
            } else if let Ok(rel) = path.strip_prefix(&root) {
                out.push(SharedString::from(rel.to_string_lossy().into_owned()));
            }
        }
    }
    out.sort();
    out
}

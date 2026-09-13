use gpui_kit::SharedString;

/// Walk the working directory for @-mention candidates (files only,
/// depth ≤ 4, skips build/VCS dirs, capped at 500 entries).
pub fn scan_project_files() -> Vec<SharedString> {
    const SKIP: [&str; 6] = ["target", ".git", "node_modules", ".idea", ".sloc-guard", "dist"];
    let root = std::env::current_dir().unwrap_or_default();
    let mut out = Vec::new();
    let mut stack = vec![(root.clone(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > 4 || out.len() >= 500 {
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

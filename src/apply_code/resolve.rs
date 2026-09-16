//! Target resolution for apply-to-file: where a code block's contents
//! should land. A first-line comment hint (`// path: <file>`, `# <file>`,
//! `-- <file>`) names the file directly; a language tag that is or carries
//! a path (` ```rust src/main.rs `) does too. Without either, the picker
//! ranks `project_files` by the tag's extension. Every candidate passes
//! `relative_path`, which refuses anything that would escape the project.

use std::path::{Component, Path};

use gpui_kit::SharedString;

/// A first-line path hint: `// path: <file>`, `# path: <file>` or
/// `-- path: <file>` name the target directly; a bare path comment
/// (`// src/foo.rs`, `# scripts/x.py`) does too. Anything else — prose,
/// shebangs, license headers — is not a hint.
pub(crate) fn path_hint(code: &str) -> Option<String> {
    let line = code.lines().next()?.trim();
    let body = ["//", "#", "--"].iter().find_map(|p| line.strip_prefix(p))?.trim();
    let candidate = body.strip_prefix("path:").map(str::trim).unwrap_or(body);
    looks_like_path(candidate).then(|| candidate.to_string())
}

/// Whether `s` reads as a project-relative file path rather than prose:
/// no whitespace, and a `.` or `/` so plain words don't count.
fn looks_like_path(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(char::is_whitespace) && (s.contains('.') || s.contains('/'))
}

/// A language tag that is itself a path (` ```src/main.rs `) or carries
/// one after the language (` ```rust src/main.rs `).
pub(crate) fn lang_path(lang: Option<&str>) -> Option<String> {
    lang?.split_whitespace().find(|tok| looks_like_path(tok)).map(str::to_string)
}

/// The file extension a language tag maps to — used to rank the picker's
/// file list. Unknown tags return `None` and leave the order alone.
pub(crate) fn lang_extension(lang: Option<&str>) -> Option<&'static str> {
    const EXTS: &[(&[&str], &str)] = &[
        (&["rust", "rs"], "rs"),
        (&["python", "py"], "py"),
        (&["javascript", "js", "jsx"], "js"),
        (&["typescript", "ts", "tsx"], "ts"),
        (&["go", "golang"], "go"),
        (&["c", "h"], "c"),
        (&["cpp", "cc", "cxx", "hpp"], "cpp"),
        (&["java"], "java"),
        (&["ruby", "rb"], "rb"),
        (&["swift"], "swift"),
        (&["kotlin", "kt"], "kt"),
        (&["css", "scss"], "css"),
        (&["html", "htm"], "html"),
        (&["json", "jsonc"], "json"),
        (&["yaml", "yml"], "yaml"),
        (&["toml"], "toml"),
        (&["xml"], "xml"),
        (&["markdown", "md"], "md"),
        (&["sql"], "sql"),
        (&["vue"], "vue"),
        (&["svelte"], "svelte"),
    ];
    let name = lang?.split_whitespace().next()?;
    EXTS.iter().find(|(names, _)| names.contains(&name)).map(|(_, ext)| *ext)
}

/// `s` as a project-relative path, or `None` when it would escape the
/// project (absolute, `..`, drive letters) or isn't a path at all.
pub(crate) fn relative_path(s: &str) -> Option<String> {
    if !looks_like_path(s) || s.contains(':') {
        return None;
    }
    let path = Path::new(s);
    if path.is_absolute() || !path.components().all(|c| matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(s.to_string())
}

/// The picker's file list: `rank_files` order with files matching the
/// block's extension floated to the top (stable — scan order survives
/// within each group).
pub(crate) fn apply_ranked(files: &[SharedString], ext: Option<&str>, query: &str) -> Vec<SharedString> {
    let mut ranked = crate::file_palette::rank_files(files, &[], query);
    if let Some(ext) = ext {
        ranked.sort_by_key(|f| usize::from(!f.ends_with(&format!(".{ext}"))));
    }
    ranked
}

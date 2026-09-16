//! Unit tests for apply-to-file target resolution (see
//! `apply_code::resolve`): comment hints, language-tag paths, project
//! confinement, and the picker's extension ranking. Headless coverage of
//! the button, picker, and approval flow lives in `apply_code_ui_tests`.
//! Narrow imports on purpose (see `composer_testutil`).

use gpui_kit::SharedString;

use crate::apply_code::{apply_ranked, lang_extension, lang_path, path_hint, relative_path};

fn files(paths: &[&str]) -> Vec<SharedString> {
    paths.iter().map(|p| SharedString::from(*p)).collect()
}

#[test]
fn path_hint_reads_comment_hints() {
    assert_eq!(path_hint("// path: src/foo.rs\nfn main() {}"), Some("src/foo.rs".to_string()));
    assert_eq!(path_hint("# path: scripts/x.py\nprint(1)"), Some("scripts/x.py".to_string()));
    assert_eq!(path_hint("-- path: db/up.sql\nselect 1"), Some("db/up.sql".to_string()));
    // A bare path comment works too.
    assert_eq!(path_hint("// src/foo.rs\nfn main() {}"), Some("src/foo.rs".to_string()));
    assert_eq!(path_hint("# scripts/x.py\nprint(1)"), Some("scripts/x.py".to_string()));
}

#[test]
fn path_hint_ignores_prose() {
    assert_eq!(path_hint("# Hello world\nprint(1)"), None);
    assert_eq!(path_hint("// Copyright 2024\nfn main() {}"), None);
    assert_eq!(path_hint("fn main() {}"), None);
    assert_eq!(path_hint(""), None);
    // Only the first line counts.
    assert_eq!(path_hint("x = 1\n# path: later.py"), None);
}

#[test]
fn lang_path_finds_paths_in_tags() {
    assert_eq!(lang_path(Some("rust src/main.rs")), Some("src/main.rs".to_string()));
    assert_eq!(lang_path(Some("src/lib.rs")), Some("src/lib.rs".to_string()));
    assert_eq!(lang_path(Some("rust")), None);
    assert_eq!(lang_path(None), None);
}

#[test]
fn lang_extension_maps_common_tags() {
    assert_eq!(lang_extension(Some("rust")), Some("rs"));
    assert_eq!(lang_extension(Some("python")), Some("py"));
    assert_eq!(lang_extension(Some("typescript")), Some("ts"));
    assert_eq!(lang_extension(Some("brainfuck")), None);
    assert_eq!(lang_extension(None), None);
}

#[test]
fn relative_path_rejects_escapes() {
    assert_eq!(relative_path("src/foo.rs"), Some("src/foo.rs".to_string()));
    assert_eq!(relative_path("/etc/passwd"), None);
    assert_eq!(relative_path("../outside.rs"), None);
    assert_eq!(relative_path("a/../b.rs"), None);
    assert_eq!(relative_path("C:/x.rs"), None);
    assert_eq!(relative_path("hello"), None);
}

#[test]
fn apply_ranked_floats_matching_extension() {
    let all = files(&["docs/guide.md", "src/main.rs", "src/lib.rs"]);
    let ranked = apply_ranked(&all, Some("rs"), "");
    assert_eq!(ranked, files(&["src/main.rs", "src/lib.rs", "docs/guide.md"]));
    // No extension → scan order.
    assert_eq!(apply_ranked(&all, None, ""), all);
    // A query still filters.
    assert_eq!(apply_ranked(&all, Some("rs"), "lib"), files(&["src/lib.rs"]));
}

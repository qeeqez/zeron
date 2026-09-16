//! Unit tests for `fs_ops`: `relative_path` validation and `perform`
//! against real temp dirs. The headless explorer flows live in
//! `fs_ops_ui_tests.rs` — split for the SLOC cap.

use std::path::{Path, PathBuf};

use crate::files::fs_ops::{FsOp, perform, relative_path};

/// A temp dir for `perform` — real disk ops, no git needed.
fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rixlcode-fsops-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn relative_path_rejects_escapes_and_absolutes() {
    assert!(relative_path("").is_none(), "empty");
    assert!(relative_path("../x").is_none(), "parent escape");
    assert!(relative_path("a/../../x").is_none(), "interior escape");
    assert!(relative_path("/abs").is_none(), "absolute");
    assert!(relative_path("a//b").is_none(), "empty segment");
    assert!(relative_path("./x").is_none(), "leading dot");
    assert!(relative_path(".").is_none(), "bare dot");
    assert!(relative_path("C:/x").is_none(), "drive letter");
    assert_eq!(relative_path("src/lib.rs"), Some(Path::new("src/lib.rs")));
    assert_eq!(relative_path("README.md"), Some(Path::new("README.md")));
    assert_eq!(relative_path(".env"), Some(Path::new(".env")), "dotfiles are normal names");
}

#[test]
fn perform_runs_each_op_on_disk() {
    let dir = temp_dir("perform");
    perform(&dir, &FsOp::NewFolder("a/b".into())).unwrap();
    assert!(dir.join("a/b").is_dir(), "NewFolder creates nested dirs");
    perform(&dir, &FsOp::NewFile("a/b/f.txt".into())).unwrap();
    assert!(dir.join("a/b/f.txt").is_file(), "NewFile creates the file");
    perform(&dir, &FsOp::Rename { old: "a/b/f.txt".into(), new: "a/b/g.txt".into() }).unwrap();
    assert!(!dir.join("a/b/f.txt").exists() && dir.join("a/b/g.txt").exists(), "Rename moves the file");
    perform(&dir, &FsOp::Delete { path: "a".into(), is_dir: true }).unwrap();
    assert!(!dir.join("a").exists(), "Delete removes the dir tree");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn perform_refuses_clobber_and_escape() {
    let dir = temp_dir("refuse");
    perform(&dir, &FsOp::NewFile("dup.txt".into())).unwrap();
    assert!(perform(&dir, &FsOp::NewFile("dup.txt".into())).is_err(), "NewFile never overwrites");
    assert!(perform(&dir, &FsOp::NewFile("../out.txt".into())).is_err(), "escape refused");
    assert!(perform(&dir, &FsOp::Delete { path: "/tmp".into(), is_dir: true }).is_err(), "absolute refused");
    assert!(perform(&dir, &FsOp::Rename { old: "dup.txt".into(), new: "../out.txt".into() }).is_err(), "rename escape refused");
    assert!(dir.join("dup.txt").exists(), "refused ops leave the tree alone");
    let _ = std::fs::remove_dir_all(&dir);
}

//! Project scope — the folder the app was opened on. Real Codex keys
//! sessions to the project: each directory gets its own chat history under
//! `~/.rixl/rixlcode/projects/<slug>-<hash>/` instead of one global list.
//!
//! The project is resolved once at launch: the first positional argument
//! that names an existing directory (`rixlcode ~/repo`, `rixlcode .`), else
//! the process cwd. `launch` also re-roots the process so the backend
//! (`codex exec` inherits cwd), `git` and the @-mention scan all run
//! against the same tree.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

/// A canonicalized directory the app runs against, plus its store dir.
#[derive(Clone)]
pub struct Project {
    root: PathBuf,
    /// Display name — the root's last component ("rixlcode").
    name: String,
    /// `~/.rixl/rixlcode/projects/<slug>-<hash>/`.
    dir: PathBuf,
}

/// Per-project UI state, persisted as `<project>/state.json`. Global
/// `Settings` stays project-agnostic; anything tied to the open folder
/// (which chat was active) lives here.
#[derive(Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectState {
    pub active_chat: usize,
}

impl Project {
    /// Resolve the launch project and make it the process cwd. The first
    /// positional argument that is an existing directory wins — a file
    /// argument opens its parent (`rixlcode README.md` still lands in the
    /// repo). With no usable argument, or when the directory cannot be
    /// entered, the cwd is the project.
    ///
    /// Resolved once per process: `enter` re-roots the cwd, so a second
    /// `Workspace` (New Window) must reuse the cached project — resolving
    /// again would interpret a relative dir arg against the new cwd and
    /// land in a nested folder (`rixlcode repo` inside `repo` → `repo/repo`).
    pub fn launch() -> Self {
        static LAUNCH: LazyLock<Project> = LazyLock::new(Project::resolve_launch);
        LAUNCH.clone()
    }

    /// The uncached resolution behind `launch` — see it for the rules.
    fn resolve_launch() -> Self {
        let requested = std::env::args().skip(1).map(PathBuf::from).find(|p| p.exists()).map(Self::open);
        match requested {
            Some(project) if project.enter().is_ok() => project,
            _ => Self::current(),
        }
    }

    /// The project at the process cwd.
    pub fn current() -> Self {
        Self::open(std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")))
    }

    /// Open `path` as a project. Canonicalizes so `a/b/../c` and `a/c`
    /// share one store; a file path resolves to its parent directory; a
    /// nonexistent path falls back to the cwd.
    pub fn open(path: impl AsRef<Path>) -> Self {
        let root = match path.as_ref().canonicalize() {
            Ok(p) if p.is_dir() => p,
            Ok(p) => p.parent().map_or_else(cwd, Path::to_path_buf),
            Err(_) => cwd(),
        };
        let name = root.file_name().and_then(|n| n.to_str()).unwrap_or("project").to_string();
        let dir = projects_dir().join(store_id(&root, &name));
        Self { root, name, dir }
    }

    /// The project root the agent runs against.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The project's display name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// This project's store directory under `~/.rixl/rixlcode/projects/`.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Directory holding this project's chat files.
    pub fn chats_dir(&self) -> PathBuf {
        self.dir().join("chats")
    }

    /// Per-project state; defaults when the file is missing or unreadable.
    pub fn load_state(&self) -> ProjectState {
        fs::read_to_string(self.dir().join("state.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persist per-project state (atomic tmp+rename, skipped when the file
    /// already matches). Also seeds `project.json` so the hash-named store
    /// dir stays self-describing.
    pub fn save_state(&self, state: &ProjectState) {
        let _ = fs::create_dir_all(self.dir());
        self.write_marker();
        let path = self.dir().join("state.json");
        let Ok(json) = serde_json::to_string_pretty(state) else { return };
        if fs::read_to_string(&path).is_ok_and(|old| old == json) {
            return;
        }
        write_atomic(&path, &json);
    }

    /// Move pre-project chats (`~/.rixl/rixlcode/chats/*.json`) into this
    /// project's store so existing users keep their history. Runs once:
    /// a project that already has chats is never clobbered, and the legacy
    /// `active_chat` setting seeds `state.json`.
    ///
    /// Resumable: a `legacy-migration` marker in the store dir records that
    /// a run started. Without it, any chat in the target means "this
    /// project has its own history" and migration stays out of the way.
    /// With it, leftover legacy files are retried — a mid-move failure
    /// can't strand chats behind the occupied check.
    pub fn migrate_legacy_chats(&self, legacy_active: usize) {
        let legacy = crate::persist::dirs_home().join(".rixl/rixlcode/chats");
        let marker = self.dir().join("legacy-migration");
        let Ok(entries) = fs::read_dir(&legacy) else {
            if !legacy.exists() {
                let _ = fs::remove_file(&marker);
            }
            return;
        };
        let mut files: Vec<PathBuf> = entries.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "json")).collect();
        if files.is_empty() {
            let _ = fs::remove_dir(&legacy);
            if !legacy.exists() {
                let _ = fs::remove_file(&marker);
            }
            return;
        }
        let target = self.chats_dir();
        let occupied =
            fs::read_dir(&target).is_ok_and(|mut d| d.any(|e| e.is_ok_and(|e| e.path().extension().is_some_and(|x| x == "json"))));
        if occupied && !marker.exists() {
            return;
        }
        files.sort();
        let _ = fs::create_dir_all(&target);
        // Written before the first move: a crash mid-run leaves the marker
        // behind so the next launch resumes instead of reading the
        // half-moved target as occupied.
        write_atomic(&marker, "");
        for path in &files {
            move_legacy(path, &target);
        }
        if !self.dir().join("state.json").exists() {
            self.save_state(&ProjectState { active_chat: legacy_active });
        }
        // Only succeeds once the legacy dir is empty — strays stay put and
        // the marker keeps the next launch retrying them.
        let _ = fs::remove_dir(&legacy);
        if !legacy.exists() {
            let _ = fs::remove_file(&marker);
        }
    }

    /// Re-root the process at the project root so the backend, `git` and
    /// any other cwd-relative work runs against the opened folder.
    fn enter(&self) -> std::io::Result<()> {
        std::env::set_current_dir(&self.root)
    }

    /// `project.json` — a small marker making the hash-named dir
    /// self-describing (and giving a future project picker its list).
    fn write_marker(&self) {
        let path = self.dir().join("project.json");
        if path.exists() {
            return;
        }
        #[derive(Serialize)]
        struct Marker<'a> {
            v: u32,
            root: &'a Path,
            name: &'a str,
        }
        if let Ok(json) = serde_json::to_string_pretty(&Marker { v: 1, root: self.root(), name: self.name() }) {
            write_atomic(&path, &json);
        }
    }
}

fn cwd() -> PathBuf {
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/"))
}

/// Move one legacy chat file into `target`. On a resume collision the slot
/// may already hold a file: identical content means a previous run moved it
/// (drop the source); different content means the slot belongs to another
/// chat (take a free index — never overwrite). Cross-device moves fall
/// back to copy-then-remove so a failed copy can't delete the only copy.
fn move_legacy(path: &Path, target: &Path) {
    let Some(name) = path.file_name() else { return };
    let mut dst = target.join(name);
    if dst.exists() {
        if fs::read(path).ok().zip(fs::read(&dst).ok()).is_some_and(|(a, b)| a == b) {
            let _ = fs::remove_file(path);
            return;
        }
        dst = free_slot(target);
    }
    if fs::rename(path, &dst).is_err() && fs::copy(path, &dst).is_ok() {
        let _ = fs::remove_file(path);
    }
}

/// First `N.json` name not taken in `dir` — used when a resumed migration
/// hits a slot that already holds a different chat.
fn free_slot(dir: &Path) -> PathBuf {
    for ix in 0usize.. {
        let candidate = dir.join(format!("{ix}.json"));
        if !candidate.exists() {
            return candidate;
        }
    }
    unreachable!()
}

fn projects_dir() -> PathBuf {
    crate::persist::dirs_home().join(".rixl/rixlcode/projects")
}

/// `<slug>-<fnv1a64>` — human-readable in Finder, collision-free across
/// same-named folders in different parents.
fn store_id(root: &Path, name: &str) -> String {
    let mut slug: String = name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect();
    while slug.contains("--") {
        slug = slug.replace("--", "-");
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "project" } else { &slug[..slug.len().min(24)] };
    format!("{slug}-{:016x}", fnv1a(root.as_os_str().as_encoded_bytes()))
}

/// Stable hash for the store id — `DefaultHasher` makes no
/// cross-version/restart guarantees, so a fixed FNV-1a it is.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in bytes {
        h = (h ^ u64::from(*b)).wrapping_mul(0x100000001b3);
    }
    h
}

fn write_atomic(path: &Path, contents: &str) {
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, contents).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

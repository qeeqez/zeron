//! The `mmdc` (mermaid-cli) side of mermaid rendering: the `MermaidRenderer`
//! trait tests fake, the real `Mmdc` process runner, and the process-wide
//! renderer slot. `mmdc` writes the source to a temp `.mmd` and produces an
//! SVG; results cache by (source, theme) hash so remounts don't re-run it.

use std::sync::Arc;
use std::time::{Duration, Instant};

/// Longest a single `mmdc` invocation may take before it is killed.
const MMDC_TIMEOUT: Duration = Duration::from_secs(15);
/// Rendered SVGs retained across remounts, keyed by (source, dark) hash.
const SVG_CACHE_MAX: usize = 64;

/// Why a diagram could not be rendered — picks the hint under the source.
#[derive(Debug)]
pub(crate) enum MermaidError {
    /// `mmdc` is not on PATH.
    Unavailable,
    /// `mmdc` failed or timed out, or the temp files misbehaved — carries
    /// stderr's first line or the io error.
    Failed(String),
}

impl From<std::io::Error> for MermaidError {
    fn from(e: std::io::Error) -> Self {
        Self::Failed(e.to_string())
    }
}

/// How mermaid source becomes SVG — the real impl shells out to `mmdc`;
/// tests substitute a fake via `set_mermaid_renderer`.
pub(crate) trait MermaidRenderer: Send + Sync {
    fn render_svg(&self, source: &str, dark: bool) -> Result<String, MermaidError>;
}

/// Renders through `mmdc` (mermaid-cli): probes for the binary, writes the
/// source to a temp `.mmd`, and runs `mmdc -i in -o out.svg`.
pub(crate) struct Mmdc {
    available: std::sync::LazyLock<bool>,
    cache: parking_lot::Mutex<std::collections::HashMap<u64, String>>,
}

impl Mmdc {
    pub(crate) fn new() -> Self {
        let probe = || {
            std::process::Command::new("mmdc")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        Self {
            available: std::sync::LazyLock::new(probe),
            cache: Default::default(),
        }
    }
}

impl MermaidRenderer for Mmdc {
    fn render_svg(&self, source: &str, dark: bool) -> Result<String, MermaidError> {
        let key = hash(&(source, dark));
        if let Some(svg) = self.cache.lock().get(&key) {
            return Ok(svg.clone());
        }
        if !*self.available {
            return Err(MermaidError::Unavailable);
        }
        let svg = run_mmdc(source, dark)?;
        let mut cache = self.cache.lock();
        if cache.len() >= SVG_CACHE_MAX {
            cache.clear();
        }
        cache.insert(key, svg.clone());
        Ok(svg)
    }
}

/// One `mmdc` run: source → temp file → SVG string. stderr goes to a file so
/// a chatty failure can't fill the pipe and deadlock the timeout poll.
fn run_mmdc(source: &str, dark: bool) -> Result<String, MermaidError> {
    let dir = std::env::temp_dir().join(format!("rixl-mermaid-{}-{:x}", std::process::id(), hash(&source)));
    let (input, output, err) = (dir.join("in.mmd"), dir.join("out.svg"), dir.join("err.txt"));
    let run = (|| {
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&input, source)?;
        let mut child = std::process::Command::new("mmdc")
            .arg("-i")
            .arg(&input)
            .arg("-o")
            .arg(&output)
            .args(["-b", "transparent", "-t", if dark { "dark" } else { "default" }, "-q"])
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(&err)?)
            .spawn()?;
        let deadline = Instant::now() + MMDC_TIMEOUT;
        loop {
            match child.try_wait()? {
                Some(status) if status.success() => return Ok(std::fs::read_to_string(&output)?),
                Some(_) => {
                    let detail = std::fs::read_to_string(&err).unwrap_or_default();
                    return Err(MermaidError::Failed(detail.lines().next().unwrap_or("mmdc failed").to_string()));
                },
                None if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                None => {
                    let _ = child.kill();
                    return Err(MermaidError::Failed("mmdc timed out".to_string()));
                },
            }
        }
    })();
    let _ = std::fs::remove_dir_all(&dir);
    run
}

/// The process-wide renderer — swapped for a fake in tests.
static RENDERER: std::sync::LazyLock<parking_lot::RwLock<Arc<dyn MermaidRenderer>>> =
    std::sync::LazyLock::new(|| parking_lot::RwLock::new(Arc::new(Mmdc::new())));

pub(crate) fn mermaid_renderer() -> Arc<dyn MermaidRenderer> {
    RENDERER.read().clone()
}

/// Install the renderer used by every subsequent mermaid block — tests only.
#[cfg(test)]
pub(crate) fn set_mermaid_renderer(renderer: Arc<dyn MermaidRenderer>) {
    *RENDERER.write() = renderer;
}

/// Cheap content key for the SVG cache and the temp dir name.
fn hash<T: std::hash::Hash + ?Sized>(value: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut h);
    std::hash::Hasher::finish(&h)
}

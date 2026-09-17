//! First-run provider detection: which provider CLIs are on PATH and
//! whether the Ollama daemon answers on its default port. The scan runs
//! on the background executor — PATH walks and TCP connects never touch
//! the UI thread — and lands on `Workspace::detected_providers`, which
//! the onboarding card (button copy) and the provider wizard (Detected
//! chips + pre-selection) read.
//!
//! The probe is behind a seam: production uses `SystemProbe`, test
//! builds default to a no-I/O `NullProbe`, and tests install fakes via
//! `set_probe` so the async spawn/land path still runs end to end.

use std::sync::Arc;

use gpui_kit::*;

use crate::providers::ProviderKind;
use crate::workspace::Workspace;

/// The environment probe behind detection — a `which`-style PATH check
/// plus a TCP reachability check for daemon kinds.
pub(crate) trait ProviderProbe: Send + Sync {
    /// The resolved path when `bin` is an executable file on PATH.
    fn cli_path(&self, bin: &str) -> Option<String>;
    /// `true` when `bin` resolves to an executable file on PATH.
    fn on_path(&self, bin: &str) -> bool {
        self.cli_path(bin).is_some()
    }
    /// `true` when a TCP connect to `port` on localhost succeeds.
    fn daemon_up(&self, port: u16) -> bool;
}

/// One instance's health: a three-level status plus the one-line reason
/// the detail panel shows ("claude found at /usr/local/bin/claude").
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProviderHealth {
    pub level: HealthLevel,
    pub reason: String,
}

/// The status-dot level: green ready, amber degraded (cli present but not
/// authed / daemon down), gray not installed or not configured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum HealthLevel {
    Ready,
    Degraded,
    Missing,
}

impl HealthLevel {
    /// Element-id suffix for the row's dot — tests assert the level.
    pub(crate) fn slug(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Degraded => "degraded",
            Self::Missing => "missing",
        }
    }
}

impl ProviderHealth {
    fn at(level: HealthLevel, reason: String) -> Self {
        Self { level, reason }
    }
}

/// Map one instance's probe answers to a health status. Pure over the
/// probe — call it on the background executor. `signed_out` is captured
/// on the UI thread (auth state lives on entities the probe can't read).
pub(crate) fn probe_health(p: &crate::providers::ProviderInstance, signed_out: bool, probe: &dyn ProviderProbe) -> ProviderHealth {
    let info = p.kind.info();
    if let Some(bin) = info.cli {
        return match probe.cli_path(bin) {
            Some(path) if signed_out => ProviderHealth::at(HealthLevel::Degraded, format!("{bin} found at {path} — not signed in")),
            Some(path) => match info.daemon {
                Some(port) if !probe.daemon_up(port) => {
                    ProviderHealth::at(HealthLevel::Degraded, format!("{bin} not listening on :{port}"))
                },
                _ => ProviderHealth::at(HealthLevel::Ready, format!("{bin} found at {path}")),
            },
            None => match info.daemon {
                Some(port) if probe.daemon_up(port) => ProviderHealth::at(HealthLevel::Ready, format!("{bin} daemon listening on :{port}")),
                Some(port) => ProviderHealth::at(HealthLevel::Missing, format!("{bin} not on PATH and not listening on :{port}")),
                None => ProviderHealth::at(HealthLevel::Missing, format!("{bin} not found on PATH")),
            },
        };
    }
    if let Some(port) = info.daemon {
        return match probe.daemon_up(port) {
            true => ProviderHealth::at(HealthLevel::Ready, format!("daemon listening on :{port}")),
            false => ProviderHealth::at(HealthLevel::Missing, format!("daemon not listening on :{port}")),
        };
    }
    if matches!(p.kind, ProviderKind::Sim) {
        return ProviderHealth::at(HealthLevel::Ready, "built in — always ready".to_string());
    }
    // http/mcp/acp: nothing installable — configured-or-not is the status.
    match p.command.trim().is_empty() {
        false => ProviderHealth::at(HealthLevel::Ready, format!("configured: {}", p.command)),
        true => ProviderHealth::at(HealthLevel::Missing, "not configured".to_string()),
    }
}

/// The real probe: PATH walk + a short `connect_timeout` to localhost.
#[cfg(not(test))]
struct SystemProbe;

#[cfg(not(test))]
impl ProviderProbe for SystemProbe {
    fn cli_path(&self, bin: &str) -> Option<String> {
        use std::os::unix::fs::PermissionsExt;
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(bin))
                .find(|candidate| candidate.is_file() && candidate.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0))
                .map(|candidate| candidate.display().to_string())
        })
    }

    fn daemon_up(&self, port: u16) -> bool {
        std::net::TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], port)), std::time::Duration::from_millis(150))
            .is_ok()
    }
}

/// Test-build default — detects nothing and performs no I/O, so mounted
/// workspaces never probe the developer's real PATH or sockets.
#[cfg(test)]
struct NullProbe;

#[cfg(test)]
impl ProviderProbe for NullProbe {
    fn cli_path(&self, _bin: &str) -> Option<String> {
        None
    }

    fn daemon_up(&self, _port: u16) -> bool {
        false
    }
}

#[cfg(not(test))]
fn default_probe() -> Arc<dyn ProviderProbe> {
    Arc::new(SystemProbe)
}

#[cfg(test)]
fn default_probe() -> Arc<dyn ProviderProbe> {
    Arc::new(NullProbe)
}

/// The process-wide probe — swapped for a fake in tests.
static PROBE: std::sync::LazyLock<parking_lot::RwLock<Arc<dyn ProviderProbe>>> =
    std::sync::LazyLock::new(|| parking_lot::RwLock::new(default_probe()));

/// Install the probe used by subsequent scans — tests only.
#[cfg(test)]
pub(crate) fn set_probe(probe: Arc<dyn ProviderProbe>) {
    *PROBE.write() = probe;
}

/// One scan of the environment: every kind whose probe matches, in
/// `ProviderKind::ALL` order. Pure over the installed probe — call it on
/// the background executor.
pub(crate) fn scan_providers() -> Vec<ProviderKind> {
    let probe = PROBE.read().clone();
    ProviderKind::ALL.into_iter().filter(|k| k.detected_by(&*probe)).collect()
}

/// One health pass over `(instance, signed_out)` pairs — the auth flag is
/// captured on the UI thread before the spawn. Pure over the installed
/// probe — call it on the background executor.
pub(crate) fn scan_health(targets: &[(crate::providers::ProviderInstance, bool)]) -> Vec<(String, ProviderHealth)> {
    let probe = PROBE.read().clone();
    targets
        .iter()
        .map(|(p, signed_out)| (p.id.clone(), probe_health(p, *signed_out, &*probe)))
        .collect()
}

impl Workspace {
    /// Kick a provider scan on the background executor — a no-op while
    /// one is in flight. The onboarding card calls this on first render
    /// and the wizard on every open; `land_detection` publishes results.
    pub(crate) fn detect_providers(&mut self, cx: &mut Context<Self>) {
        if self.detection_pending {
            return;
        }
        self.detection_pending = true;
        let task = cx.background_executor().spawn(async move { scan_providers() });
        cx.spawn(async move |this, cx| {
            let found = task.await;
            let _ = this.update(cx, |this, cx| this.land_detection(found, cx));
        })
        .detach();
    }

    /// Publish a finished scan: re-render the workspace (the card's
    /// button copy reads `detected_providers`) and forward the result to
    /// an open wizard for its chips and pre-selection. The panel update
    /// is deferred — it re-reads the workspace for instance ids, which
    /// can't happen while this borrow is still held.
    fn land_detection(&mut self, found: Vec<ProviderKind>, cx: &mut Context<Self>) {
        self.detection_pending = false;
        self.detected_providers = Some(found.clone());
        let panel = self.settings_panel.downgrade();
        cx.defer(move |cx| {
            let _ = panel.update_in(cx, |panel, window, cx| panel.apply_detected(found, window, cx));
        });
        cx.notify();
    }
}

#[cfg(test)]
#[path = "provider_detect_tests.rs"]
mod provider_detect_tests;

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
    /// `true` when `bin` resolves to an executable file on PATH.
    fn on_path(&self, bin: &str) -> bool;
    /// `true` when a TCP connect to `port` on localhost succeeds.
    fn daemon_up(&self, port: u16) -> bool;
}

/// The real probe: PATH walk + a short `connect_timeout` to localhost.
#[cfg(not(test))]
struct SystemProbe;

#[cfg(not(test))]
impl ProviderProbe for SystemProbe {
    fn on_path(&self, bin: &str) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::env::var_os("PATH").is_some_and(|path| {
            std::env::split_paths(&path).any(|dir| {
                let candidate = dir.join(bin);
                candidate.is_file() && candidate.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
            })
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
    fn on_path(&self, _bin: &str) -> bool {
        false
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

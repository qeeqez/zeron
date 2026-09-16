//! Headless tests for mermaid diagram blocks: fence detection, the
//! missing-`mmdc` fallback (fake renderer), the rendered diagram's Copy-SVG
//! and "View source" toggle, and the mid-stream unclosed fence.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::Root;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ElementId, Entity, TestAppContext, VisualTestContext};

use crate::views::mermaid_mmdc::{MermaidError, MermaidRenderer, set_mermaid_renderer};
use crate::workspace::Workspace;

/// Mount a `Workspace` in a headless window with `HOME` redirected to a
/// temp dir so settings/chats stay off the real profile.
fn mount(cx: &mut TestAppContext) -> (Entity<Workspace>, &mut VisualTestContext) {
    let dir = std::env::temp_dir().join(format!("rixlcode-mmd-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    unsafe { std::env::set_var("HOME", &dir) };
    cx.update(gpui_kit::init);
    let mut ws = None;
    let (root, cx) = cx.add_window_view(|window, cx| {
        let view = cx.new(|cx| Workspace::new(window, cx));
        ws = Some(view.clone());
        Root::new(view, window, cx)
    });
    let _ = root;
    (ws.unwrap(), cx)
}

fn seed(ws: &Entity<Workspace>, text: &str, cx: &mut VisualTestContext) {
    ws.update(cx, |this, cx| this.push_note(text.to_string(), cx));
}

/// ElementIds registered by `.test_support()` in the last frame.
fn observed_ids(window: &gpui_kit::Window) -> Vec<ElementId> {
    snapshots(window).iter().filter_map(|s| s.path().last().cloned()).collect()
}

fn has_id_containing(ids: &[ElementId], needle: &str) -> bool {
    ids.iter().any(|id| format!("{id:?}").contains(needle))
}

/// The aria label on the mermaid hint element, if it rendered this frame.
fn hint_label(window: &gpui_kit::Window) -> Option<String> {
    snapshots(window)
        .iter()
        .find(|s| s.path().last().is_some_and(|id| format!("{id:?}").contains("mermaid-hint-")))
        .and_then(|s| s.label().map(str::to_string))
}

/// Fake `mmdc`: `svg = None` reports the binary missing; `Some` returns it.
/// `calls` counts render attempts so tests can prove nothing ran.
struct FakeMmdc {
    svg: Option<&'static str>,
    calls: Arc<AtomicUsize>,
}

impl MermaidRenderer for FakeMmdc {
    fn render_svg(&self, _source: &str, _dark: bool) -> Result<String, MermaidError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.svg {
            Some(svg) => Ok(svg.to_string()),
            None => Err(MermaidError::Unavailable),
        }
    }
}

fn install_fake(svg: Option<&'static str>) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    set_mermaid_renderer(Arc::new(FakeMmdc { svg, calls: calls.clone() }));
    calls
}

const FLOWCHART: &str = "graph TD\n    A[Start] --> B[End]";
const FAKE_SVG: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 40"><rect width="100" height="40" fill="#eee"/><text x="10" y="25">A to B</text></svg>"##;

/// A ` ```mermaid ` fence parses into the custom block — the mermaid label
/// and block element render instead of a plain code block.
#[test]
fn mermaid_fence_renders_as_diagram_block() {
    let mut app = TestAppContext::single();
    install_fake(None);
    let (ws, cx) = mount(&mut app);
    seed(&ws, &format!("Here:\n\n```mermaid\n{FLOWCHART}\n```\n"), cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        assert!(has_id_containing(&ids, "mermaid-block-0-"), "mermaid block missing: {ids:?}");
        assert!(has_id_containing(&ids, "code-lang-0-mermaid"), "mermaid label missing: {ids:?}");
    });
}

/// A non-mermaid fence still takes the plain code-block path — the custom
/// parser must not swallow it.
#[test]
fn non_mermaid_fence_stays_a_code_block() {
    let mut app = TestAppContext::single();
    install_fake(None);
    let (ws, cx) = mount(&mut app);
    seed(&ws, "```rust\nfn main() {}\n```\n", cx);
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        assert!(has_id_containing(&ids, "code-lang-0-rust"), "rust label missing: {ids:?}");
        assert!(!has_id_containing(&ids, "mermaid-block-"), "mermaid block leaked: {ids:?}");
    });
}

/// Without `mmdc` the block keeps the source and shows the install hint —
/// the fake renderer's `Unavailable` is the mocked probe.
#[test]
fn missing_mmdc_shows_source_with_install_hint() {
    let mut app = TestAppContext::single();
    install_fake(None);
    let (ws, cx) = mount(&mut app);
    seed(&ws, &format!("```mermaid\n{FLOWCHART}\n```\n"), cx);
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        assert!(has_id_containing(&ids, "mermaid-block-0-"), "block missing: {ids:?}");
        assert!(!has_id_containing(&ids, "mermaid-img-"), "diagram should not render: {ids:?}");
        let hint = hint_label(window).expect("install hint missing");
        assert!(hint.contains("mermaid-cli"), "hint: {hint:?}");
    });
}

/// With `mmdc` the fence renders the rasterized diagram plus Copy-SVG and
/// the "View source" toggle.
#[test]
fn rendered_diagram_shows_image_and_controls() {
    let mut app = TestAppContext::single();
    install_fake(Some(FAKE_SVG));
    let (ws, cx) = mount(&mut app);
    seed(&ws, &format!("```mermaid\n{FLOWCHART}\n```\n"), cx);
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        assert!(has_id_containing(&ids, "mermaid-img-0-"), "diagram missing: {ids:?}");
        assert!(has_id_containing(&ids, "mermaid-copy-svg-0-"), "Copy SVG missing: {ids:?}");
        assert!(has_id_containing(&ids, "mermaid-toggle-0-"), "toggle missing: {ids:?}");
        assert!(!has_id_containing(&ids, "mermaid-hint-"), "hint should not show: {ids:?}");
    });
}

/// Copy-SVG writes the rendered markup; the toggle flips to the mermaid
/// source and back.
#[test]
fn copy_svg_and_view_source_toggle() {
    let mut app = TestAppContext::single();
    install_fake(Some(FAKE_SVG));
    let (ws, cx) = mount(&mut app);
    seed(&ws, &format!("```mermaid\n{FLOWCHART}\n```\n"), cx);
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        assert!(has_id_containing(&ids, "mermaid-img-0-"), "diagram missing: {ids:?}");
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let copy_id = observed_ids(window)
            .into_iter()
            .find(|id| format!("{id:?}").contains("mermaid-copy-svg-0-"))
            .expect("Copy SVG missing");
        window.click(copy_id, cx);
        let clip = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
        assert!(clip.contains("<svg"), "clipboard: {clip:?}");
        let toggle_id = observed_ids(window)
            .into_iter()
            .find(|id| format!("{id:?}").contains("mermaid-toggle-0-"))
            .expect("toggle missing");
        window.click(toggle_id, cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        assert!(!has_id_containing(&ids, "mermaid-img-0-"), "source view should hide the diagram: {ids:?}");
        assert!(has_id_containing(&ids, "mermaid-toggle-0-"), "toggle missing in source view: {ids:?}");
        let copy_id = observed_ids(window)
            .into_iter()
            .find(|id| format!("{id:?}").contains("copy-code-0-"))
            .expect("copy button missing in source view");
        window.click(copy_id, cx);
        let clip = cx.read_from_clipboard().and_then(|item| item.text()).unwrap_or_default();
        assert!(clip.contains("graph TD"), "clipboard: {clip:?}");
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        let toggle_id = ids
            .iter()
            .find(|id| format!("{id:?}").contains("mermaid-toggle-0-"))
            .cloned()
            .expect("toggle missing in source view");
        window.click(toggle_id, cx);
    });
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        assert!(has_id_containing(&ids, "mermaid-img-0-"), "diagram should return: {ids:?}");
    });
}

/// Mid-stream the fence is unclosed: the block shows the source and never
/// kicks a render — the fake's call count stays at zero.
#[test]
fn unclosed_fence_never_invokes_mmdc() {
    let mut app = TestAppContext::single();
    let calls = install_fake(Some(FAKE_SVG));
    let (ws, cx) = mount(&mut app);
    seed(&ws, &format!("```mermaid\n{FLOWCHART}"), cx);
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        assert!(has_id_containing(&ids, "mermaid-block-0-"), "block missing: {ids:?}");
        assert!(!has_id_containing(&ids, "mermaid-img-"), "unclosed fence rendered: {ids:?}");
        assert!(!has_id_containing(&ids, "mermaid-hint-"), "hint should not show mid-stream: {ids:?}");
    });
    assert_eq!(calls.load(Ordering::SeqCst), 0, "mmdc ran for an unclosed fence");
}

/// The real `Mmdc` path end to end: a fake `mmdc` shell script on PATH
/// answers the `--version` probe and writes an SVG for `-o`. Each nextest
/// test is its own process, so mutating PATH is safe.
#[test]
fn real_mmdc_path_renders_via_fake_binary() {
    let dir = std::env::temp_dir().join(format!("rixlcode-fake-mmdc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bin = dir.join("mmdc");
    std::fs::write(
        &bin,
        "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 1.0.0; exit 0; fi\n\
         while [ $# -gt 0 ]; do [ \"$1\" = \"-o\" ] && { shift; printf '%s' \
         '<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 10\"></svg>' > \"$1\"; }; shift; done\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let path = std::env::var("PATH").unwrap_or_default();
    unsafe { std::env::set_var("PATH", format!("{}:{path}", dir.display())) };

    let mut app = TestAppContext::single();
    set_mermaid_renderer(Arc::new(crate::views::mermaid_mmdc::Mmdc::new()));
    let (ws, cx) = mount(&mut app);
    seed(&ws, &format!("```mermaid\n{FLOWCHART}\n```\n"), cx);
    cx.run_until_parked();
    cx.update(|window, cx| {
        window.draw(cx).clear(cx);
        let ids = observed_ids(window);
        assert!(has_id_containing(&ids, "mermaid-img-0-"), "real mmdc path should render: {ids:?}");
        assert!(!has_id_containing(&ids, "mermaid-hint-"), "hint should not show: {ids:?}");
    });
}

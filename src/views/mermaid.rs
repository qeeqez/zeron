//! Mermaid diagram blocks: a ` ```mermaid ` fence parses into a custom
//! Markdown node (wired in `markdown.rs`) and renders through `mmdc`
//! (mermaid-cli) when it is installed — the SVG is rasterized once into a
//! `RenderImage` so painting never re-rasterizes. Without `mmdc` the block
//! keeps its code look plus a hint line; a rendered diagram offers Copy-SVG
//! and a "View source" toggle back to the code.
//!
//! The `mmdc` call sits behind `MermaidRenderer` so tests inject a fake — the
//! real `Mmdc` shells out and caches SVGs by (source, theme) hash.

use std::hash::{Hash, Hasher};
use std::sync::Arc;

use gpui_kit::assets::IconName;
use gpui_kit::base::ObservedElement;
use gpui_kit::base::text::{MarkdownNode, MarkdownParseContext, markdown_ast as mdast};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use super::mermaid_mmdc::{MermaidError, mermaid_renderer};

/// Custom node name the parser tags mermaid fences with.
const NODE_NAME: &str = "mermaid";

/// Cheap content key for keyed state.
fn hash<T: Hash + ?Sized>(value: &T) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    value.hash(&mut h);
    h.finish()
}

/// Turn a ` ```mermaid ` code node into a custom block. `closed` records
/// whether the fence is terminated — mid-stream the block re-parses on every
/// delta, and only a closed fence is worth a `mmdc` run.
pub(crate) fn parse_block(node: &mdast::Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode> {
    let mdast::Node::Code(code) = node else { return None };
    if !code.lang.as_deref().is_some_and(|l| l.trim().eq_ignore_ascii_case("mermaid")) {
        return None;
    }
    let source = cx.node_source(node);
    let closed = source.is_some_and(|s| s.trim_end().ends_with("```") || s.trim_end().ends_with("~~~"));
    Some(
        MarkdownNode::new(NODE_NAME, closed)
            .text(code.value.clone())
            .markdown(source.unwrap_or(&code.value).to_string()),
    )
}

/// Per-block render state: the async `mmdc` result plus the view-source
/// toggle. Keyed by (message, code hash) so an edited block re-renders.
struct MermaidView {
    status: Status,
    show_source: bool,
}

enum Status {
    /// Fence still open, or the render task has not been kicked yet.
    Unstarted,
    Rendering,
    Done {
        image: Arc<RenderImage>,
        svg: SharedString,
    },
    /// The hint line shown under the source fallback.
    Failed(SharedString),
}

/// Render a mermaid block: kick `mmdc` once the fence is closed, then show
/// the diagram — or the source with a hint when rendering can't run.
pub(crate) fn render_block(ix: usize, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> AnyElement {
    let code = node.as_text().to_string();
    let closed = node.data::<bool>().copied().unwrap_or(false);
    let key = hash(&code);
    let state = window.use_keyed_state(ElementId::Name(format!("mermaid-{ix}-{key:x}").into()), cx, |_, _| MermaidView {
        status: Status::Unstarted,
        show_source: false,
    });
    if closed && matches!(state.read(cx).status, Status::Unstarted) {
        state.update(cx, |s, _| s.status = Status::Rendering);
        let (dark, renderer, svg_renderer) = (cx.theme().is_dark(), mermaid_renderer(), cx.svg_renderer());
        let weak = state.downgrade();
        let source = code.clone();
        cx.spawn(async move |cx| {
            // Rasterize once on the background executor — the RenderImage is
            // what `img` paints, so frames never re-rasterize.
            let result = cx
                .background_executor()
                .spawn(async move {
                    let svg = renderer.render_svg(&source, dark)?;
                    let image = svg_renderer
                        .render_single_frame(svg.as_bytes(), 1.0)
                        .map_err(|e| MermaidError::Failed(e.to_string()))?;
                    Ok((image, svg))
                })
                .await;
            let _ = weak.update(cx, |s, cx| {
                s.status = match result {
                    Ok((image, svg)) => Status::Done { image, svg: svg.into() },
                    Err(MermaidError::Unavailable) => Status::Failed("Install mermaid-cli (mmdc) to render diagrams".into()),
                    Err(MermaidError::Failed(msg)) => Status::Failed(format!("Couldn't render diagram — {msg}").into()),
                };
                cx.notify();
            });
        })
        .detach();
    }
    let s = state.read(cx);
    let (done, show_source, hint) = match &s.status {
        Status::Done { image, svg } => (Some((image.clone(), svg.clone())), s.show_source, None),
        Status::Failed(msg) => (None, s.show_source, Some(msg.clone())),
        _ => (None, s.show_source, None),
    };
    let theme = cx.theme();
    let (accent, muted, mono_family, mono_size) =
        (theme.accent, theme.muted_foreground, theme.mono_font_family.clone(), theme.mono_font_size);
    // The Custom-node wrapper shrink-wraps, so controls live in a header row
    // inside the block — an absolute overlay would escape the block's bounds
    // and its buttons would miss clicks.
    let header = {
        let d = div()
            .flex()
            .items_center()
            .gap_2()
            .pb_2()
            .text_xs()
            .text_color(muted)
            .child(div().id(ElementId::Name(format!("code-lang-{ix}-mermaid").into())).test_support().child("mermaid"));
        let d = match &done {
            Some((_, svg)) if !show_source => d.child(copy_svg_button(ix, key, svg.clone())),
            _ => d,
        };
        let d = match done.is_some() {
            true => d.child(toggle_button(ix, key, &state, show_source)),
            false => d,
        };
        d.child(super::markdown::copy_code_button(ix, key, code.clone(), window, cx))
    };
    let body: AnyElement = match done {
        Some((image, _)) if !show_source => diagram(ix, key, image).into_any_element(),
        _ => div().child(code.clone()).into_any_element(),
    };
    let block = div()
        .id(ElementId::Name(format!("mermaid-block-{ix}-{key:x}").into()))
        .test_support()
        .w_full()
        .min_w_0()
        .p_3()
        .bg(accent)
        .font_family(mono_family)
        .text_size(mono_size)
        .child(header)
        .child(body);
    div()
        .w_full()
        .min_w_0()
        .child(block)
        .when_some(hint, |d, hint| {
            d.child(
                div()
                    .id(ElementId::Name(format!("mermaid-hint-{ix}-{key:x}").into()))
                    .test_support()
                    .aria_label(hint.to_string())
                    .pt_1()
                    .text_xs()
                    .text_color(muted)
                    .child(hint),
            )
        })
        .into_any_element()
}

/// The diagram, capped at its natural width and the column's, aspect kept.
fn diagram(ix: usize, key: u64, image: Arc<RenderImage>) -> ObservedElement<Stateful<Div>> {
    let nat = image.size(0).map(|d| d.0 as f32 / SMOOTH_SVG_SCALE_FACTOR);
    let mut frame = div().id(ElementId::Name(format!("mermaid-img-{ix}-{key:x}").into())).test_support().min_w_0();
    if nat.width > 0. && nat.height > 0. {
        // The Custom-node wrapper shrink-wraps, so a definite width is what
        // actually sizes the frame; `max_w_full` clamps it to the column.
        frame = frame.w(px(nat.width)).max_w_full().aspect_ratio(nat.width / nat.height);
    }
    frame.child(img(ImageSource::Render(image)).size_full().object_fit(ObjectFit::Contain))
}

/// "Copy SVG" — writes the rendered markup to the clipboard.
fn copy_svg_button(ix: usize, key: u64, svg: SharedString) -> ObservedElement<Stateful<Div>> {
    div()
        .id(ElementId::Name(format!("mermaid-copy-svg-{ix}-{key:x}").into()))
        .test_support()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_1()
        .child(IconName::Image)
        .child("Copy SVG")
        .on_click(move |_, _, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(svg.to_string()));
        })
}

/// Flip between the rendered diagram and the mermaid source.
fn toggle_button(ix: usize, key: u64, state: &Entity<MermaidView>, show_source: bool) -> ObservedElement<Stateful<Div>> {
    let state = state.clone();
    div()
        .id(ElementId::Name(format!("mermaid-toggle-{ix}-{key:x}").into()))
        .test_support()
        .cursor_pointer()
        .flex()
        .items_center()
        .gap_1()
        .child(if show_source { IconName::ChartNetwork } else { IconName::CodeXml })
        .child(if show_source { "View diagram" } else { "View source" })
        .on_click(move |_, _, cx| {
            state.update(cx, |s, cx| {
                s.show_source = !s.show_source;
                cx.notify();
            });
        })
}

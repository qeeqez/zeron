//! The `< N/M >` version pager on an assistant reply with alternatives —
//! `<` swaps in the previous (older) version, `>` steps forward to the
//! newest. Always visible: it carries state, like the pinned bookmark
//! icon, so it doesn't hide with the hover-revealed ghost actions.

use gpui_kit::assets::IconName;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::views::cards::MsgCtx;
use crate::workspace::Workspace;

/// One pager chevron's inputs — dimmed and inert at the chain's ends.
struct PagerBtn {
    id: (&'static str, usize),
    icon: IconName,
    enabled: bool,
    color: Hsla,
    ws: Entity<Workspace>,
    ix: usize,
    older: bool,
}

fn pager_btn(p: PagerBtn) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    let btn = div()
        .id(p.id)
        .test_support()
        .text_xs()
        .text_color(if p.enabled { p.color } else { p.color.opacity(0.35) })
        .child(p.icon);
    if p.enabled {
        let ws = p.ws;
        let ix = p.ix;
        let older = p.older;
        btn.cursor_pointer().on_click(move |_, _, cx| {
            ws.update(cx, |this, cx| this.cycle_alternative(ix, older, cx));
        })
    } else {
        btn
    }
}

/// `< pos/total >` — position counts versions newest-first, so a fresh
/// regenerate shows `1/2` and paging back reaches the original reply.
/// Compact density drops the position label, leaving bare chevrons.
pub(super) fn version_pager(mc: MsgCtx, ws: &Entity<Workspace>, compact: bool) -> Div {
    let MsgCtx { ix, msg, .. } = mc;
    let muted = hsla(0.0, 0.0, 0.55, 1.0);
    let pos = msg.version_position();
    let total = msg.alternatives.len() + 1;
    div()
        .flex()
        .items_center()
        .child(pager_btn(PagerBtn {
            id: ("ver-prev", ix),
            icon: IconName::ChevronLeft,
            enabled: pos < total,
            color: muted,
            ws: ws.clone(),
            ix,
            older: true,
        }))
        .when(!compact, |d| {
            d.child(
                div()
                    .id(("ver-pos", ix))
                    .test_support()
                    .aria_label(format!("{pos}/{total}"))
                    .text_xs()
                    .text_color(muted)
                    .child(format!("{pos}/{total}")),
            )
        })
        .child(pager_btn(PagerBtn {
            id: ("ver-next", ix),
            icon: IconName::ChevronRight,
            enabled: pos > 1,
            color: muted,
            ws: ws.clone(),
            ix,
            older: false,
        }))
}

//! The composer's `/` menu row — split from `composer_helpers.rs` so both
//! stay under the SLOC cap.

use gpui_kit::assets::IconName;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

/// One slash-menu row: the command, its icon and help line, and whether a
/// running turn disables it (`/compact` starts a turn — mid-reply it would
/// queue like text, so the row sits out until the turn ends).
pub struct SlashSpec {
    pub cmd: &'static str,
    pub icon: IconName,
    pub desc: &'static str,
    pub disabled: bool,
}

pub fn slash_item(spec: SlashSpec, ws: &Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let SlashSpec { cmd, icon, desc, disabled } = spec;
    let ws = ws.clone();
    let cmd_str = cmd.to_string();
    let row = div()
        .id(SharedString::from(format!("slash-{cmd}")))
        .test_support()
        .px_2()
        .py_1()
        .rounded_md()
        .text_sm()
        .child(
            div().flex().items_center().gap_2().child(icon).child(format!("/{cmd}")).child(
                div()
                    .id(SharedString::from(format!("slash-{cmd}-desc")))
                    .test_support()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(desc.to_string()),
            ),
        );
    if disabled {
        row.text_color(cx.theme().muted_foreground)
    } else {
        row.cursor_pointer().hover(|d| d.bg(cx.theme().accent)).on_click(move |_, window, cx| {
            apply_slash(&ws, &cmd_str, window, cx);
        })
    }
}

fn apply_slash(ws: &Entity<Workspace>, cmd: &str, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| this.run_command(cmd, window, cx));
}

//! The chat find bar's element — split from `crate::chat_find` (which owns
//! the state, matching and navigation) for the SLOC cap. The bar is a
//! `Workspace` method because it reads `find` state directly, same as the
//! terminal find bar in `views::terminal_find`.

use gpui_kit::accesskit::Toggled;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::Input;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Disableable, Sizable};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::chat_search::find_opts::FindOpts;
use crate::workspace::Workspace;

/// The bar's two flag chips — each knows its element id, icon, a11y label,
/// which `FindOpts` flag it reads and the `Workspace` method a click calls.
#[derive(Clone, Copy)]
enum FindChip {
    /// Match Case — the "Aa" icon, `case_sensitive`.
    Case,
    /// Whole Word — `whole_word`.
    Word,
}

impl FindChip {
    fn id(self) -> &'static str {
        match self {
            Self::Case => "find-match-case",
            Self::Word => "find-whole-word",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Case => IconName::CaseSensitive,
            Self::Word => IconName::WholeWord,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Case => "Match case",
            Self::Word => "Whole word",
        }
    }

    fn on(self, opts: FindOpts) -> bool {
        match self {
            Self::Case => opts.case_sensitive,
            Self::Word => opts.whole_word,
        }
    }

    fn toggle(self, ws: &mut Workspace, cx: &mut Context<Workspace>) {
        match self {
            Self::Case => ws.find_match_case_toggle(cx),
            Self::Word => ws.find_whole_word_toggle(cx),
        }
    }
}

/// One find toggle chip: a small bordered icon button that carries the
/// accent fill while on — the same look as the changes panel's
/// ignore-whitespace chip. Checkbox role + `aria_toggled` expose the state
/// to tests and assistive tech.
fn find_toggle(chip: FindChip, opts: FindOpts, cx: &mut Context<Workspace>) -> impl IntoElement {
    let theme = cx.theme();
    let on = chip.on(opts);
    let mut el = div()
        .id(chip.id())
        .test_support()
        .role(gpui_kit::Role::CheckBox)
        .aria_toggled(if on { Toggled::True } else { Toggled::False })
        .aria_label(chip.label())
        .cursor_pointer()
        .px_1p5()
        .py_0p5()
        .rounded_md()
        .border_1()
        .border_color(theme.border)
        .child(chip.icon());
    el = if on {
        el.bg(theme.accent).text_color(theme.accent_foreground)
    } else {
        el.text_color(theme.muted_foreground)
    };
    el.on_click(cx.listener(move |this, _, _, cx| chip.toggle(this, cx)))
}

impl Workspace {
    /// The Cmd-F find bar: query input, the Match Case / Whole Word toggle
    /// chips, the All / You / Assistant role cycle, `n / total` readout,
    /// prev/next and close. Enter/Shift+Enter in the input reach `find_jump`
    /// through the input's `PressEnter` event; Esc reaches `find_escape`
    /// via the chat column's `EscapeKey` listener.
    pub(crate) fn find_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let total = self.find_matches(cx).len();
        let current = if total == 0 { 0 } else { self.find.match_ix.min(total - 1) + 1 };
        div()
            .id("find-bar")
            .test_support()
            .flex()
            .items_center()
            .gap_2()
            .px_4()
            .py_1()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(IconName::Search)
            .child(div().flex_1().child(Input::new(&self.find.input).appearance(true)))
            .child(find_toggle(FindChip::Case, self.find.opts, cx))
            .child(find_toggle(FindChip::Word, self.find.opts, cx))
            .child(
                Button::new("find-role")
                    .ghost()
                    .xsmall()
                    .label(self.find.role.label())
                    .on_click(cx.listener(|this, _, _, cx| this.find_role_cycle(cx))),
            )
            .child(
                div()
                    .id("find-count")
                    .test_support()
                    .aria_label(format!("{current} / {total}"))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{current} / {total}")),
            )
            .child(
                Button::new("find-prev")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronUp)
                    .disabled(total == 0)
                    .on_click(cx.listener(|this, _, _, cx| this.find_jump(true, cx))),
            )
            .child(
                Button::new("find-next")
                    .ghost()
                    .xsmall()
                    .icon(IconName::ChevronDown)
                    .disabled(total == 0)
                    .on_click(cx.listener(|this, _, _, cx| this.find_jump(false, cx))),
            )
            .child(
                Button::new("find-close")
                    .ghost()
                    .xsmall()
                    .icon(IconName::X)
                    .on_click(cx.listener(|this, _, window, cx| this.open_chat_find(window, cx))),
            )
    }
}

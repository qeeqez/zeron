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

/// The bar's two flag chips — each knows its icon, a11y label and which
/// `FindOpts` flag it drives. Shared with the terminal find bar and the
/// global-search filter row, which reuse the same styling.
#[derive(Clone, Copy)]
pub(crate) enum FindChip {
    /// Match Case — the "Aa" icon, `case_sensitive`.
    Case,
    /// Whole Word — `whole_word`.
    Word,
}

/// Which surface a chip sits on — picks the element id and, for the
/// workspace find bars, the toggle methods a click calls. `Search` is the
/// global-search filter row: the dialog renders while the workspace is
/// leased, so its click writes the filters entity instead of calling a
/// `Workspace` method.
#[derive(Clone, Copy)]
pub(crate) enum FindScope {
    Chat,
    Term,
    Search,
}

impl FindChip {
    fn id(self, scope: FindScope) -> &'static str {
        match (scope, self) {
            (FindScope::Chat, Self::Case) => "find-match-case",
            (FindScope::Chat, Self::Word) => "find-whole-word",
            (FindScope::Term, Self::Case) => "term-find-match-case",
            (FindScope::Term, Self::Word) => "term-find-whole-word",
            (FindScope::Search, Self::Case) => "search-match-case",
            (FindScope::Search, Self::Word) => "search-whole-word",
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

    /// The flag this chip reads.
    pub(crate) fn on(self, opts: FindOpts) -> bool {
        match self {
            Self::Case => opts.case_sensitive,
            Self::Word => opts.whole_word,
        }
    }

    /// Flip the flag this chip drives — used where a `Workspace` listener
    /// can't reach (the global-search row writes the filters entity).
    pub(crate) fn flip(self, opts: &mut FindOpts) {
        match self {
            Self::Case => opts.case_sensitive = !opts.case_sensitive,
            Self::Word => opts.whole_word = !opts.whole_word,
        }
    }

    fn toggle(self, ws: &mut Workspace, scope: FindScope, cx: &mut Context<Workspace>) {
        match (scope, self) {
            (FindScope::Chat, Self::Case) => ws.find_match_case_toggle(cx),
            (FindScope::Chat, Self::Word) => ws.find_whole_word_toggle(cx),
            (FindScope::Term, Self::Case) => ws.term_find_match_case_toggle(cx),
            (FindScope::Term, Self::Word) => ws.term_find_whole_word_toggle(cx),
            (FindScope::Search, _) => {},
        }
    }
}

/// The toggle chip's shell: a small bordered icon button carrying the
/// accent fill while on — the same look as the changes panel's
/// ignore-whitespace chip. Checkbox role + `aria_toggled` expose the state
/// to tests and assistive tech. The caller attaches the click handler —
/// a workspace listener in the find bars, a filters-entity update in the
/// global-search row.
pub(crate) fn toggle_chip(chip: FindChip, scope: FindScope, on: bool, cx: &App) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    let theme = cx.theme();
    let el = div()
        .id(chip.id(scope))
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
    if on {
        el.bg(theme.accent).text_color(theme.accent_foreground)
    } else {
        el.text_color(theme.muted_foreground)
    }
}

/// A find toggle chip wired to a workspace bar — `scope` picks the ids
/// and the `Workspace` toggle method the click calls.
pub(crate) fn find_toggle(chip: FindChip, scope: FindScope, opts: FindOpts, cx: &mut Context<Workspace>) -> impl IntoElement {
    toggle_chip(chip, scope, chip.on(opts), cx).on_click(cx.listener(move |this, _, _, cx| chip.toggle(this, scope, cx)))
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
            .child(find_toggle(FindChip::Case, FindScope::Chat, self.find.opts, cx))
            .child(find_toggle(FindChip::Word, FindScope::Chat, self.find.opts, cx))
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

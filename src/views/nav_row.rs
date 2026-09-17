//! The one sidebar row: icon + label (or a custom body) + optional suffix and
//! context menu. Both the chat list (`sidebar_row.rs`) and the settings nav
//! (`settings_nav.rs`) render through this component, so padding, colors,
//! hover and selected states can never drift apart.
//!
//! The styling mirrors gpui-component's `SidebarMenuItem`, extended with what
//! the app rows need: a caller-chosen element id, a hover group for
//! reveal-on-hover suffixes, a body slot (the chat rename editor replaces the
//! label), and an optional context menu.

use std::rc::Rc;

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type RowContent = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;
type MenuBuilder = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;
/// Drag sources erase their payload type so `NavRow` stays non-generic — the
/// closure applies the typed `on_drag` to the row's `Stateful<Div>` at render.
type DragSource = Rc<dyn Fn(Stateful<Div>) -> Stateful<Div>>;

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu};
use gpui_kit::component::sidebar::SidebarItem;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Collapsible, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

/// A clickable sidebar row. `id` is the row's element id — tests and callers
/// address the padded row itself (`settings-nav-<section>`, `("chat-row", n)`).
#[derive(Clone)]
pub(crate) struct NavRow {
    id: ElementId,
    icon: Option<IconName>,
    label: SharedString,
    active: bool,
    /// Multi-select member: a subtler accent wash than `active` — the
    /// sidebar's bulk-op selection, not the open chat.
    selected: bool,
    /// False drops the hover wash — the settings nav's "No settings match"
    /// placeholder is a label, not a clickable row.
    hoverable: bool,
    collapsed: bool,
    /// Hover-reveal group name painted on the row's wrapper (e.g. the chat
    /// row's "…" button shows on `group_hover`).
    group: Option<SharedString>,
    on_click: Option<ClickHandler>,
    /// Replaces the label + suffix entirely (the inline rename editor).
    body: Option<RowContent>,
    suffix: Option<RowContent>,
    /// Extra content between the icon and the label — the chat row's color
    /// dot lives here. Rendered only in the expanded row.
    leading: Option<RowContent>,
    /// A 2px colored bar on the row's left edge — the folder color tag's
    /// grouping cue on member chat rows.
    edge: Option<Hsla>,
    /// Drag source applied to the row's `Stateful<Div>` — the toolkit's 2px
    /// threshold keeps plain clicks from starting a drag.
    on_drag: Option<DragSource>,
    /// Drop target applied the same way — the closure registers the typed
    /// `on_drag_move`/`on_drop` pair on the stateful row.
    drop_target: Option<DragSource>,
    /// Drop indicator edge while a chat drag hovers this row — `Some(true)`
    /// draws the line on the row's top edge, `Some(false)` the bottom.
    drop_line: Option<bool>,
    context_menu: Option<MenuBuilder>,
}

impl NavRow {
    pub(crate) fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            icon: None,
            label: label.into(),
            active: false,
            selected: false,
            hoverable: true,
            collapsed: false,
            group: None,
            leading: None,
            edge: None,
            on_click: None,
            body: None,
            suffix: None,
            context_menu: None,
            on_drag: None,
            drop_target: None,
            drop_line: None,
        }
    }

    pub(crate) fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Selected state: medium weight on the sidebar-accent fill.
    pub(crate) fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Multi-select member: a faint accent wash under the hover state —
    /// distinct from `active`'s solid selected-chat fill.
    pub(crate) fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Marks the wrapper as a hover group so suffixes can reveal on hover.
    pub(crate) fn group(mut self, group: impl Into<SharedString>) -> Self {
        self.group = Some(group.into());
        self
    }

    pub(crate) fn on_click(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Rc::new(handler));
        self
    }

    /// Hover wash on/off — off for non-interactive placeholder rows.
    pub(crate) fn hoverable(mut self, hoverable: bool) -> Self {
        self.hoverable = hoverable;
        self
    }

    /// Drag source: `value` is the payload drop targets downcast to, `preview`
    /// builds the view that follows the cursor. Mirrors `Div::on_drag`; the
    /// payload must be `Clone` because the erased closure is `Fn`, not `FnOnce`.
    pub(crate) fn on_drag<T, W>(
        mut self, value: T, preview: impl Fn(&T, Point<Pixels>, &mut Window, &mut App) -> Entity<W> + 'static,
    ) -> Self
    where
        T: Clone + 'static,
        W: 'static + Render,
    {
        let preview = Rc::new(preview);
        self.on_drag = Some(Rc::new(move |row| {
            let preview = preview.clone();
            row.on_drag(value.clone(), move |value, offset, window, cx| preview(value, offset, window, cx))
        }));
        self
    }

    /// Drop target for drags of type `T`: `on_move` runs on every drag move
    /// (the toolkit fires it for all registered rows — check `ev.bounds`
    /// against `ev.event.position`), `on_drop` on release over this row.
    pub(crate) fn drop_target<T>(
        mut self, on_move: impl Fn(&DragMoveEvent<T>, &mut Window, &mut App) + 'static,
        on_drop: impl Fn(&T, &mut Window, &mut App) + 'static,
    ) -> Self
    where
        T: 'static,
    {
        let on_move = Rc::new(on_move);
        let on_drop = Rc::new(on_drop);
        self.drop_target = Some(Rc::new(move |row| {
            let on_drop = on_drop.clone();
            row.on_drag_move({
                let on_move = on_move.clone();
                move |ev: &DragMoveEvent<T>, window, cx| on_move(ev, window, cx)
            })
            .on_drop(move |value: &T, window, cx| on_drop(value, window, cx))
        }));
        self
    }

    /// The drop indicator edge — see the `drop_line` field.
    pub(crate) fn drop_line(mut self, edge: Option<bool>) -> Self {
        self.drop_line = edge;
        self
    }

    /// Custom row content replacing the label and suffix.
    pub(crate) fn body<E: IntoElement>(mut self, body: impl Fn(&mut Window, &mut App) -> E + 'static) -> Self {
        self.body = Some(Rc::new(move |window, cx| body(window, cx).into_any_element()));
        self
    }

    /// Trailing content — status dot, spinner, hover-revealed menu button.
    pub(crate) fn suffix<E: IntoElement>(mut self, suffix: impl Fn(&mut Window, &mut App) -> E + 'static) -> Self {
        self.suffix = Some(Rc::new(move |window, cx| suffix(window, cx).into_any_element()));
        self
    }

    /// Leading content between the icon and the label — the chat row's
    /// color dot. Skipped while the sidebar is collapsed.
    pub(crate) fn leading<E: IntoElement>(mut self, leading: impl Fn(&mut Window, &mut App) -> E + 'static) -> Self {
        self.leading = Some(Rc::new(move |window, cx| leading(window, cx).into_any_element()));
        self
    }

    /// A 2px colored bar on the row's left edge — the folder color tag's
    /// grouping cue on member chat rows.
    pub(crate) fn edge_accent(mut self, color: Hsla) -> Self {
        self.edge = Some(color);
        self
    }

    pub(crate) fn context_menu(mut self, menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static) -> Self {
        self.context_menu = Some(Rc::new(menu));
        self
    }
}

impl Collapsible for NavRow {
    fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
}

impl SidebarItem for NavRow {
    fn render(self, id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> impl IntoElement {
        // Copy the theme values up front — `theme()` borrows `cx`, and the
        // body/suffix closures need `&mut App` below.
        let (radius, accent, accent_bg, accent_fg, drag_border) = {
            let theme = cx.theme();
            (theme.radius, theme.sidebar_accent, theme.tokens.sidebar_accent, theme.sidebar_accent_foreground, theme.tokens.drag_border)
        };
        // Row content: the custom body (rename editor) replaces label+suffix.
        let content = match self.body {
            Some(body) => body(window, cx),
            None => h_flex()
                .flex_1()
                .gap_x_2()
                .justify_between()
                .overflow_x_hidden()
                .child(h_flex().flex_1().overflow_x_hidden().child(self.label.clone()))
                .when_some(self.suffix, |this, suffix| this.child(suffix(window, cx)))
                .into_any_element(),
        };
        let leading = self.leading.map(|leading| leading(window, cx));
        // The folder-color edge bar — built before `row` moves `self.id`.
        // Painted after the row so it stays visible over the active fill;
        // the vertical inset clears the row's rounded corners.
        let edge = self.edge.map(|color| {
            div()
                .id(format!("{}-folder-edge", self.id))
                .test_support()
                .absolute()
                .left_0()
                .top(px(4.))
                .bottom(px(4.))
                .w(px(2.))
                .rounded_full()
                .bg(color)
                .into_any_element()
        });
        // The drop indicator — a 2px line on the row's top or bottom edge,
        // half-overlapping so it reads as the gap the drop lands in.
        let drop_line = self.drop_line.map(|above| {
            div()
                .id(format!("{}-drop-line", self.id))
                .test_support()
                .absolute()
                .left_0()
                .right_0()
                .h(px(2.))
                .bg(drag_border)
                .map(|this| if above { this.top(px(-1.)) } else { this.bottom(px(-1.)) })
                .into_any_element()
        });
        let row = h_flex()
            .size_full()
            .id(self.id)
            // The drag listener needs the stateful element — `test_support`
            // wraps it in `Observed` under the test feature.
            .when_some(self.on_drag, |this, on_drag| on_drag(this))
            .when_some(self.drop_target, |this, drop_target| drop_target(this))
            .test_support()
            .overflow_x_hidden()
            .flex_shrink_0()
            .when(self.selected && !self.active, |this| this.bg(accent.opacity(0.5)))
            .p_2()
            .gap_x_2()
            .rounded(radius)
            .text_sm()
            .when(!self.active && self.hoverable, |this| this.hover(|this| this.bg(accent.opacity(0.8)).text_color(accent_fg)))
            .when(self.active, |this| this.font_medium().bg(accent_bg).text_color(accent_fg))
            .when_some(self.icon, |this, icon| this.child(icon))
            .when(self.collapsed, |this| this.justify_center())
            .when(!self.collapsed, |this| this.h_7().when_some(leading, |this, leading| this.child(leading)).child(content))
            .when_some(self.on_click, |this, on_click| this.on_click(move |ev, window, cx| on_click(ev, window, cx)))
            .map(|this| match self.context_menu {
                Some(menu) => this.context_menu(move |m, window, cx| menu(m, window, cx)).into_any_element(),
                None => this.into_any_element(),
            });
        div()
            .id(id)
            .test_support()
            .w_full()
            .relative()
            .when_some(self.group, |this, group| this.group(group))
            .child(row)
            .when_some(edge, |this, edge| this.child(edge))
            .when_some(drop_line, |this, line| this.child(line))
    }
}

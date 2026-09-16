//! Sidebar chat groups. `ChatGroup` mirrors gpui-component's `SidebarGroup`
//! but adds what folders need: a clickable, right-clickable header with a
//! collapse chevron. `chat_groups` builds the list — folders first
//! (alphabetical, collapsible), then "Unfiled", then "Archived"; with no
//! folders the flat Pinned/Today/Previous 7 Days/Older groups render
//! unchanged.

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_kit::component::sidebar::SidebarItem;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Collapsible, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::views::nav_row::NavRow;
use crate::views::sidebar_row::ChatDrag;
use crate::workspace::Workspace;

type ClickHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type MenuBuilder = Rc<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;

/// One sidebar section: a small header row over `NavRow` children. Folder
/// groups set `on_toggle`/`folded` so the header collapses the section;
/// plain groups (Unfiled, Archived, the recency buckets) render the same
/// header without a chevron or click.
#[derive(Clone)]
pub(super) struct ChatGroup {
    label: SharedString,
    icon: Option<IconName>,
    folded: bool,
    collapsed: bool,
    on_toggle: Option<ClickHandler>,
    context_menu: Option<MenuBuilder>,
    children: Vec<NavRow>,
    /// Folder name this header files dropped chats under — `""` for Unfiled.
    chat_drop: Option<(Entity<Workspace>, SharedString)>,
}

impl ChatGroup {
    pub(super) fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: label.into(),
            icon: None,
            folded: false,
            collapsed: false,
            on_toggle: None,
            context_menu: None,
            children: Vec::new(),
            chat_drop: None,
        }
    }

    pub(super) fn icon(mut self, icon: IconName) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Folder groups: `folded` hides the children, `on_toggle` flips it.
    pub(super) fn folded(mut self, folded: bool) -> Self {
        self.folded = folded;
        self
    }

    pub(super) fn on_toggle(mut self, handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(handler));
        self
    }

    pub(super) fn context_menu(mut self, menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static) -> Self {
        self.context_menu = Some(Rc::new(menu));
        self
    }

    pub(super) fn children(mut self, children: impl IntoIterator<Item = NavRow>) -> Self {
        self.children.extend(children);
        self
    }

    pub(super) fn child(mut self, child: NavRow) -> Self {
        self.children.push(child);
        self
    }

    /// Drop target for chat drags: releasing a `ChatDrag` on this header files
    /// the chat under `folder` (`""` unfiles). `drag_over` paints the header
    /// while a chat drag hovers it.
    pub(super) fn chat_drop_target(mut self, ws: &Entity<Workspace>, folder: impl Into<SharedString>) -> Self {
        self.chat_drop = Some((ws.clone(), folder.into()));
        self
    }
}

impl Collapsible for ChatGroup {
    fn is_collapsed(&self) -> bool {
        self.collapsed
    }

    fn collapsed(mut self, collapsed: bool) -> Self {
        self.collapsed = collapsed;
        self
    }
}

impl SidebarItem for ChatGroup {
    fn render(self, id: impl Into<ElementId>, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let id = id.into();
        let (radius, fg, muted_fg) = {
            let theme = cx.theme();
            (theme.radius, theme.sidebar_foreground.opacity(0.7), theme.muted_foreground)
        };
        let header = h_flex()
            .id(format!("group-header-{}", self.label))
            .test_support()
            .flex_shrink_0()
            .px_2()
            .rounded(radius)
            .text_xs()
            .text_color(fg)
            .h_8()
            .gap_1()
            .items_center()
            .when_some(self.icon, |this, icon| this.child(icon))
            .child(div().flex_1().overflow_x_hidden().child(self.label.clone()))
            .when_some(self.on_toggle, |this, on_toggle| {
                this.cursor_pointer()
                    .hover(|this| this.bg(muted_fg.opacity(0.2)))
                    .child(if self.folded { IconName::ChevronRight } else { IconName::ChevronDown })
                    .on_click(move |ev, window, cx| on_toggle(ev, window, cx))
            })
            .when_some(self.chat_drop, |this, (ws, folder)| {
                this.drag_over::<ChatDrag>(|style, _, _, cx| style.bg(cx.theme().tokens.drop_target)).on_drop(
                    move |drag: &ChatDrag, _window, cx| {
                        ws.update(cx, |this, cx| this.set_chat_folder(drag.id, folder.as_ref(), cx));
                    },
                )
            })
            .map(|this| match self.context_menu {
                Some(menu) => this.context_menu(move |m, window, cx| menu(m, window, cx)).into_any_element(),
                None => this.into_any_element(),
            });
        v_flex()
            .id(id.clone())
            .relative()
            .when(!self.collapsed, |this| this.child(header))
            .when(!self.folded, |this| {
                this.child(
                    div().gap_2().flex_col().children(self.children.into_iter().enumerate().map(|(ix, child)| {
                        child.collapsed(self.collapsed).render(format!("{}-{}", id, ix), window, cx).into_any_element()
                    })),
                )
            })
    }
}

/// The folder header's right-click menu — rename rewrites member chats,
/// delete unfiles them.
fn folder_menu(ws: &Entity<Workspace>, name: &str, menu: PopupMenu) -> PopupMenu {
    let ws_rename = ws.clone();
    let ws_delete = ws.clone();
    let rename_to = name.to_string();
    let delete = name.to_string();
    menu.item(PopupMenuItem::new("Rename folder").icon(IconName::Pencil).on_click(move |_, w, cx| {
        ws_rename.update(cx, |this, cx| this.open_rename_folder_dialog(&rename_to, w, cx));
    }))
    .item(PopupMenuItem::new("Delete folder").icon(IconName::Delete).on_click(move |_, _w, cx| {
        ws_delete.update(cx, |this, cx| this.delete_folder(&delete, cx));
    }))
}

/// The sidebar's group list. Folders lead (collapsible, alphabetical);
/// unfiled chats sit under "Unfiled" and archived under "Archived". With
/// no folders the flat recency groups render — same shape as before
/// folders existed. `state` is the workspace read — `row_of` already
/// holds the `cx` borrow, so this can't take one.
pub(super) fn chat_groups(
    ws: &Entity<Workspace>, state: &Workspace, filtered: &[usize], archived: &[usize], row_of: &mut impl FnMut(usize) -> NavRow,
) -> Vec<ChatGroup> {
    let folders = state.folder_names();
    let mut groups: Vec<ChatGroup> = Vec::new();
    if folders.is_empty() {
        for (bucket_ix, name) in ["Pinned", "Today", "Previous 7 Days", "Older"].iter().enumerate() {
            let items: Vec<NavRow> = filtered
                .iter()
                .copied()
                .filter(|ix| state.chat_bucket(*ix) == bucket_ix)
                .map(&mut *row_of)
                .collect();
            if !items.is_empty() {
                groups.push(ChatGroup::new(*name).children(items));
            }
        }
    } else {
        for name in &folders {
            let items: Vec<NavRow> = filtered.iter().copied().filter(|ix| state.chats[*ix].folder == *name).map(&mut *row_of).collect();
            if items.is_empty() {
                continue;
            }
            let folded = state.collapsed_folders.contains(name);
            let toggle = name.clone();
            let menu_name = name.clone();
            let ws_toggle = ws.clone();
            let ws_menu = ws.clone();
            groups.push(
                ChatGroup::new(name.clone())
                    .icon(IconName::Folder)
                    .folded(folded)
                    .on_toggle(move |_, _, cx| ws_toggle.update(cx, |this, cx| this.toggle_folder(&toggle, cx)))
                    .context_menu(move |menu, _window, _cx| folder_menu(&ws_menu, &menu_name, menu))
                    .children(items)
                    .chat_drop_target(ws, name.clone()),
            );
        }
        let unfiled: Vec<NavRow> = filtered.iter().copied().filter(|ix| state.chats[*ix].folder.is_empty()).map(&mut *row_of).collect();
        if !unfiled.is_empty() {
            groups.push(ChatGroup::new("Unfiled").chat_drop_target(ws, "").children(unfiled));
        }
    }
    if !archived.is_empty() {
        groups.push(ChatGroup::new("Archived").children(archived.iter().copied().map(row_of)));
    }
    groups
}

use gpui_kit::assets::IconName;
use gpui_kit::base::StyledExt;
use gpui_kit::component::input::Input;
use gpui_kit::component::sidebar::{Sidebar, SidebarCollapsible, SidebarGroup};

use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::workspace::Workspace;

/// Which list the sidebar shows — the chat list or the file explorer.
/// Settings replaces the whole column regardless of the tab.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SidebarTab {
    #[default]
    Chats,
    Files,
}

impl SidebarTab {
    /// Element id of the tab's header button.
    fn id(self) -> &'static str {
        match self {
            Self::Chats => "tab-chats",
            Self::Files => "tab-files",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Chats => "Chats",
            Self::Files => "Files",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Self::Chats => IconName::MessageSquare,
            Self::Files => IconName::FolderTree,
        }
    }
}

impl Workspace {
    pub fn render_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let collapsed = self.sidebar_collapsed;
        let tab = self.sidebar_tab;
        let header = div()
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            // Title row + search sit below the window's drag strip.
            .child(div().flex().items_center().gap_2().text_sm().font_bold().child(IconName::Bot).child("Rixl Code"))
            // Chats/Files tab strip — the explorer lives in the same column
            // as the chat list, like Codex's sidebar.
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(tab_button(SidebarTab::Chats, tab, cx))
                    .child(tab_button(SidebarTab::Files, tab, cx)),
            )
            // The chat search only makes sense on the Chats tab.
            .when(tab == SidebarTab::Chats, |d| d.child(self.search_row(cx)));

        let new_chat = crate::views::nav_row::NavRow::new("new-chat", "New chat")
            .icon(IconName::Plus)
            .on_click(cx.listener(|this, _, _, cx| this.new_chat(cx)));

        // Resume past threads — only backends with session support (codex)
        // get the row; others never see the affordance.
        let resume_open = self.resume_open;
        let resume = self.backend.supports_sessions().then(|| {
            crate::views::nav_row::NavRow::new("resume-toggle", "Resume")
                .icon(IconName::RotateCcw)
                .suffix(move |_, _| if resume_open { IconName::ChevronDown } else { IconName::ChevronRight })
                .on_click(cx.listener(|this, _, _, cx| this.toggle_resume(cx)))
        });

        let query = self.search.read(cx).value().to_lowercase();
        let filtered = self.sidebar_order(&query);
        let archived: Vec<usize> = (0..self.chats.len())
            .filter(|ix| self.chats[*ix].archived && (query.is_empty() || self.chats[*ix].title.to_lowercase().contains(&query)))
            .collect();
        let bucket = |ix: usize| self.chat_bucket(ix);
        let mut row_of = |ix: usize| super::sidebar_row::chat_row(&self.chats[ix], ix, self, cx);

        let group_names = ["Pinned", "Today", "Previous 7 Days", "Older"];
        let mut groups: Vec<SidebarGroup<crate::views::nav_row::NavRow>> = Vec::new();
        for (bucket_ix, name) in group_names.iter().enumerate() {
            let items: Vec<crate::views::nav_row::NavRow> =
                filtered.iter().copied().filter(|ix| bucket(*ix) == bucket_ix).map(&mut row_of).collect();
            if !items.is_empty() {
                groups.push(SidebarGroup::new(*name).children(items));
            }
        }
        if !archived.is_empty() {
            let items: Vec<crate::views::nav_row::NavRow> = archived.iter().copied().map(row_of).collect();
            groups.push(SidebarGroup::new("Archived").children(items));
        }

        // The Resume section leads the list when open — past sessions sit
        // above the chat groups like Codex's history view.
        if self.resume_open {
            let items: Vec<crate::views::nav_row::NavRow> = self
                .sessions
                .iter()
                .enumerate()
                .map(|(ix, s)| {
                    let session = s.clone();
                    // The thread's directory basename — sessions can come
                    // from other projects, so the row says where it ran.
                    let dir: SharedString = std::path::Path::new(&s.cwd)
                        .file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_default()
                        .into();
                    crate::views::nav_row::NavRow::new(("resume-session", ix), s.title.clone())
                        .icon(IconName::FileText)
                        .suffix(move |_, cx| div().text_xs().text_color(cx.theme().muted_foreground).child(dir.clone()))
                        .on_click(cx.listener(move |this, _, window, cx| this.open_session(&session, window, cx)))
                })
                .collect();
            let label = if self.sessions_loading { "Resume — loading…" } else { "Resume" };
            groups.insert(0, SidebarGroup::new(label).children(items));
        }

        let mut actions = SidebarGroup::new("").child(new_chat);
        if let Some(resume) = resume {
            actions = actions.child(resume);
        }

        let footer = div()
            .flex()
            .items_center()
            .gap_2()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(IconName::CircleUser)
            .child("Local")
            .child(div().flex_1())
            .child(
                div()
                    .id("clear-chats")
                    .cursor_pointer()
                    .child(IconName::Trash)
                    .on_click(cx.listener(|this, _, window, cx| this.clear_all_chats(window, cx))),
            )
            .child(
                div()
                    .id("settings-btn")
                    .test_support()
                    .cursor_pointer()
                    .child(IconName::Settings)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_settings(window, cx);
                    })),
            );

        div()
            .id("sidebar-wrap")
            .test_support()
            .h_full()
            .relative()
            .flex()
            .flex_col()
            .bg(crate::appearance::sidebar_fill(cx.theme(), self.sidebar_frosted))
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
            // Full-height sidebar: its top strip is a transparent window-drag
            // zone of TOP_BAR_H that lines up with the content's titlebar —
            // the traffic lights + toggle overlay its top-left.
            .child(crate::window::titlebar_drag(div().id("sidebar-titlebar").h(px(crate::window::TOP_BAR_H)).w_full()).test_support())
            .child(
                div().flex_1().min_h_0().child(
                    // One shared sidebar column: settings swaps in its nav;
                    // otherwise the tab strip picks the chat list or the
                    // file explorer.
                    if self.settings_open {
                        crate::views::settings_nav::settings_nav(
                            crate::views::settings_nav::SettingsNav {
                                panel: &self.settings_panel,
                                width: px(self.sidebar_width),
                                collapsed,
                            },
                            window,
                            cx,
                        )
                            .into_any_element()
                    } else {
                        match tab {
                            SidebarTab::Chats => {
                                // The component paints its own opaque `tokens.sidebar`
                                // — clear it so the wrap's fill (translucent when
                                // frosted) shows through.
                                //
                                // `collapsible(None)`, not `Offcanvas`: collapse is
                                // handled by unmounting the sidebar in `render`, and
                                // Offcanvas wraps the column in a 200ms width
                                // transition that restarts on every mouse-move of a
                                // resize drag — the edge chases the cursor and the
                                // view reads as shifting left/right.
                                Sidebar::new("sidebar")
                                    .w(px(self.sidebar_width))
                                    .collapsible(SidebarCollapsible::None)
                                    .collapsed(collapsed)
                                    .bg(transparent_black())
                                    .header(header)
                                    .child(actions)
                                    .children(groups)
                                    .footer(footer)
                                    .into_any_element()
                            }
                            SidebarTab::Files => self
                                .render_explorer(header.into_any_element(), footer.into_any_element(), cx)
                                .into_any_element(),
                        }
                    },
                ),
            )
            .when(!collapsed, |d| {
                d.child(
                    div()
                        .id("sidebar-resize")
                        .test_support()
                        .absolute()
                        // Below the titlebar drag strip: the strip's top
                        // TOP_BAR_H is a window-move zone, so the handle must
                        // not cover it or a grab near the top would fight the
                        // window drag.
                        .top(px(crate::window::TOP_BAR_H))
                        .right_0()
                        .bottom_0()
                        .w(px(5.))
                        .cursor_col_resize()
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, _, cx| {
                                this.resizing_sidebar = true;
                                cx.notify();
                            }),
                        ),
                )
            })
    }

    /// Switch the sidebar between the chat list and the file explorer.
    pub fn set_sidebar_tab(&mut self, tab: SidebarTab, cx: &mut Context<Self>) {
        self.sidebar_tab = tab;
        cx.notify();
    }

    /// The chat-list search field — only mounted on the Chats tab.
    fn search_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .items_center()
            .gap_1()
            .child(div().flex_1().child(Input::new(&self.search).prefix(IconName::Search).appearance(true)))
            .when(!self.search.read(cx).value().is_empty(), |d| {
                d.child(
                    div()
                        .id("search-clear")
                        .cursor_pointer()
                        .text_color(cx.theme().muted_foreground)
                        .child(IconName::X)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.search.update(cx, |s, cx| s.set_value("", window, cx));
                        })),
                )
            })
    }
}

/// One Chats/Files tab button in the sidebar header — a segmented pair that
/// mirrors the settings nav's accent-on-active styling.
fn tab_button(tab: SidebarTab, current: SidebarTab, cx: &mut Context<Workspace>) -> impl IntoElement {
    let active = tab == current;
    div()
        .id(tab.id())
        .test_support()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .gap_1()
        .py_1()
        .rounded_md()
        .text_xs()
        .cursor_pointer()
        .when(active, |d| d.font_medium().bg(cx.theme().accent))
        .when(!active, |d| d.text_color(cx.theme().muted_foreground).hover(|d| d.bg(cx.theme().muted)))
        .child(tab.icon())
        .child(tab.label())
        .on_click(cx.listener(move |this, _, _, cx| this.set_sidebar_tab(tab, cx)))
}

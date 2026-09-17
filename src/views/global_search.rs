//! The Cmd-Shift-F global-search dialog's view layer: the filter row (Date,
//! Model, Provider chips), the result rows, and the `Command` element that
//! hosts them. The searchable model — `SearchDoc`/`SearchHit`/`search` —
//! lives in `crate::global_search`; declared from `views::mod` because
//! `main.rs` is at the SLOC cap.
//!
//! The dialog builds while the workspace entity is leased, so nothing here
//! may read `Workspace` — the filters live in their own entity for exactly
//! that reason (see `Workspace::search_filters`).

use std::rc::Rc;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::command::{Command, CommandGroup, CommandItem, CommandState};
use gpui_kit::component::menu::{DropdownMenu, PopupMenu, PopupMenuItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Sizable, WindowExt, h_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::chat_search::role_filter::RoleFilter;
use crate::global_search::{DateRange, SearchDoc, SearchFilters, SearchHit, search};
use crate::workspace::Workspace;

/// One filter chip: an xsmall ghost button whose label is the current
/// selection, opening `menu` on click. `id` keeps it findable in tests.
fn chip(
    id: &'static str, icon: IconName, label: SharedString,
    menu: impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static,
) -> impl IntoElement {
    Button::new(id).ghost().xsmall().icon(icon).label(label).dropdown_caret(true).dropdown_menu(menu)
}

/// The Date chip's menu — one checked row per `DateRange` preset.
fn date_menu(filters: Entity<SearchFilters>, menu: PopupMenu, _window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let current = filters.read(cx).date;
    [DateRange::Any, DateRange::Day, DateRange::Week, DateRange::Month]
        .into_iter()
        .fold(menu, |menu, range| {
            let filters = filters.clone();
            menu.item(PopupMenuItem::new(range.label()).checked(range == current).on_click(move |_, _, cx| {
                filters.update(cx, |f, _| f.date = range);
            }))
        })
}

/// The Role chip's menu — "Any role" plus the two message roles, checked
/// on the current selection. Mirrors the find bar's All / You / Assistant
/// toggle.
fn role_menu(filters: Entity<SearchFilters>, menu: PopupMenu, _window: &mut Window, cx: &mut Context<PopupMenu>) -> PopupMenu {
    let current = filters.read(cx).role;
    [RoleFilter::All, RoleFilter::User, RoleFilter::Assistant].into_iter().fold(menu, |menu, role| {
        let filters = filters.clone();
        let label = if role == RoleFilter::All { "Any role" } else { role.label() };
        menu.item(PopupMenuItem::new(label).checked(role == current).on_click(move |_, _, cx| {
            filters.update(cx, |f, _| f.role = role);
        }))
    })
}

/// The Model chip's menu — "Any model" plus every distinct model id in the
/// searched docs, checked on the current selection.
fn model_menu(
    filters: Entity<SearchFilters>, models: Rc<Vec<String>>, menu: PopupMenu, _window: &mut Window, cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let current = filters.read(cx).model.clone();
    let clear = filters.clone();
    let menu = menu.item(PopupMenuItem::new("Any model").checked(current.is_none()).on_click(move |_, _, cx| {
        clear.update(cx, |f, _| f.model = None);
    }));
    models.iter().fold(menu, |menu, model| {
        let filters = filters.clone();
        let pick = model.clone();
        menu.item(
            PopupMenuItem::new(model.clone())
                .checked(current.as_ref() == Some(model))
                .on_click(move |_, _, cx| {
                    filters.update(cx, |f, _| f.model = Some(pick.clone()));
                }),
        )
    })
}

/// The Provider chip's menu — "Any provider" plus every distinct provider
/// id in the searched docs, checked on the current selection.
fn provider_menu(
    filters: Entity<SearchFilters>, providers: Rc<Vec<String>>, menu: PopupMenu, _window: &mut Window, cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let current = filters.read(cx).provider.clone();
    let clear = filters.clone();
    let menu = menu.item(PopupMenuItem::new("Any provider").checked(current.is_none()).on_click(move |_, _, cx| {
        clear.update(cx, |f, _| f.provider = None);
    }));
    providers.iter().fold(menu, |menu, p| {
        let filters = filters.clone();
        let pick = p.clone();
        menu.item(PopupMenuItem::new(p.clone()).checked(current.as_ref() == Some(p)).on_click(move |_, _, cx| {
            filters.update(cx, |f, _| f.provider = Some(pick.clone()));
        }))
    })
}

/// The filter row above the search field: Date/Model/Provider chips whose
/// menus write the filters entity — the workspace observes it and
/// re-renders, so results narrow live. Model/Provider options are the
/// distinct stamps across `docs`; a chat with an empty stamp simply never
/// offers that value.
fn filter_row(filters: &Entity<SearchFilters>, docs: &[SearchDoc], cx: &mut App) -> AnyElement {
    let models: Rc<Vec<String>> = Rc::new(
        docs.iter()
            .map(|d| d.model.clone())
            .filter(|m| !m.is_empty())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
    );
    let providers: Rc<Vec<String>> = Rc::new(
        docs.iter()
            .map(|d| d.provider.clone())
            .filter(|p| !p.is_empty())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect(),
    );
    let current = filters.read(cx);
    let date_label: SharedString = current.date.label().into();
    let model_label: SharedString = current.model.clone().unwrap_or_else(|| "Model".into()).into();
    let provider_label: SharedString = current.provider.clone().unwrap_or_else(|| "Provider".into()).into();
    let role_label: SharedString = if current.role == RoleFilter::All { "Role".into() } else { current.role.label().into() };
    let (f_date, f_model, f_provider, f_role) = (filters.clone(), filters.clone(), filters.clone(), filters.clone());
    h_flex()
        .id("search-filters")
        .test_support()
        .gap_1()
        .px_3()
        .py_1()
        .child(chip("search-filter-date", IconName::Calendar, date_label, move |menu, window, cx| {
            date_menu(f_date.clone(), menu, window, cx)
        }))
        .child(chip("search-filter-model", IconName::Cpu, model_label, move |menu, window, cx| {
            model_menu(f_model.clone(), models.clone(), menu, window, cx)
        }))
        .child(chip("search-filter-provider", IconName::Server, provider_label, move |menu, window, cx| {
            provider_menu(f_provider.clone(), providers.clone(), menu, window, cx)
        }))
        .child(chip("search-filter-role", IconName::User, role_label, move |menu, window, cx| role_menu(f_role.clone(), menu, window, cx)))
        .into_any_element()
}

/// A result row: chat title over the match snippet over one line of
/// neighboring-message context ("You: …" / "Rixl: …"), relative age over
/// the hit's provider·model stamp at the trailing edge.
fn hit_item(row: usize, hit: SearchHit) -> CommandItem {
    let title = hit.title.clone();
    let snippet = hit.snippet.clone();
    // The neighboring message's role + text — what identifies the hit
    // without opening the chat.
    let context: Option<SharedString> = hit.context.map(|(role, text)| {
        let who = match role {
            crate::model::Role::User => "You",
            crate::model::Role::Assistant => "Rixl",
        };
        format!("{who}: {text}").into()
    });
    // Which backend produced the hit — muted under the timestamp.
    let stamps: SharedString = [hit.provider.as_str(), hit.model.as_str()]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
        .into();
    CommandItem::new().label(hit.title).child(move |_, cx| {
        h_flex()
            .flex_1()
            .gap_2()
            .items_center()
            .child(IconName::MessageSquare)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_sm().whitespace_nowrap().text_ellipsis().child(title.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .whitespace_nowrap()
                            .text_ellipsis()
                            .child(snippet.clone()),
                    )
                    .when_some(context.clone(), |d, line| {
                        d.child(
                            div()
                                .id(("hit-context", row))
                                .test_support()
                                .aria_label(line.clone())
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .whitespace_nowrap()
                                .text_ellipsis()
                                .child(line),
                        )
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_end()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(crate::palette_items::rel_time(hit.at)),
                    )
                    .when(!stamps.is_empty(), |d| d.child(div().text_xs().text_color(cx.theme().muted_foreground).child(stamps.clone()))),
            )
    })
}

/// The dialog's `Command` element, rebuilt on every workspace render —
/// `on_query` notifies so each keystroke re-runs `search` with the live
/// query against the docs snapshot, and the workspace's observe on
/// `filters` re-runs it when a chip changes a selection.
pub(crate) fn search_command(
    state: &Entity<CommandState>, docs: &[SearchDoc], filters: &Entity<SearchFilters>, ws: &Entity<Workspace>, cx: &mut App,
) -> Command {
    let ws_confirm = ws.clone();
    let ws_query = ws.clone();
    let filter_docs = docs.to_vec();
    let filter_state = filters.clone();
    let hits = search(docs, &state.read(cx).query(cx), filters.read(cx));
    let group = CommandGroup::new()
        .label("Messages")
        .items(hits.into_iter().enumerate().map(|(row, hit)| hit_item(row, hit)));
    Command::new(state)
        .placeholder("Search all chats…")
        // Matching happens in `search`, not the component's substring filter.
        .filterable(false)
        .header(move |_, _, cx| filter_row(&filter_state, &filter_docs, cx))
        .group(group)
        .empty(|state, _, cx| {
            let hint = if state.query(cx).trim().is_empty() { "Search messages across every chat" } else { "No matches" };
            div().py_6().w_full().text_center().text_sm().text_color(cx.theme().muted_foreground).child(hint)
        })
        .footer(|_, _, cx| crate::palette::command_footer("↵ open chat", cx))
        .on_query(move |_, _, cx| {
            ws_query.update(cx, |_, cx| cx.notify());
        })
        .on_confirm(move |path, window, cx| {
            ws_confirm.update(cx, |this, cx| this.confirm_global_hit(path, window, cx));
        })
        .on_cancel(|window, cx| window.close_dialog(cx))
}

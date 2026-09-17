//! Command-palette contents: the command table plus fuzzy-ranked chat
//! navigation, built fresh from the live query on every dialog render.
//!
//! The palette is `filterable(false)` — gpui-component's built-in filter is
//! substring-only, so ranking happens here and the `Command` renders exactly
//! the groups this module supplies. Everything below is pure: the dialog
//! builder runs while `Workspace::render` holds the entity lease, so it works
//! from a `ChatSnapshot` captured at open time instead of reading the
//! workspace.

use gpui_kit::assets::IconName;
use gpui_kit::component::command::{CommandGroup, CommandItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{IndexPath, h_flex};
use gpui_kit::*;

use crate::workspace::Workspace;

/// What confirming a command does. `Dispatch` goes through the normal action
/// system (and gets its keybinding hint rendered by the row). `Run` executes
/// inside `on_confirm` — after the palette dialog closes — for commands that
/// open their own surface (rename dialog, chat search, shortcuts sheet) or a
/// native prompt (export), where a post-dispatch close would hit the wrong
/// window layer or steal back focus.
pub(crate) enum Effect {
    Dispatch(Box<dyn Action>),
    Run(fn(&mut Workspace, &mut Window, &mut Context<Workspace>)),
}

pub(crate) struct CommandSpec {
    pub label: &'static str,
    pub icon: IconName,
    /// Extra terms the fuzzy matcher also scores.
    pub keywords: &'static [&'static str],
    pub effect: Effect,
}

/// A chat row's data, snapshotted when the palette opens.
#[derive(Clone)]
pub(crate) struct ChatSnapshot {
    pub id: u64,
    pub title: SharedString,
    pub active: bool,
    pub at: std::time::SystemTime,
}

/// One palette row: a workspace command or a jump to a chat.
pub(crate) enum Entry {
    Command(CommandSpec),
    Chat(ChatSnapshot),
}

impl Workspace {
    /// Chats in sidebar order (pinned, then recency) as palette rows —
    /// captured at open so the dialog builder never reads the workspace.
    pub(crate) fn palette_chats(&self) -> Vec<ChatSnapshot> {
        self.sidebar_order("")
            .into_iter()
            .map(|ix| {
                let chat = &self.chats[ix];
                ChatSnapshot {
                    id: chat.id,
                    title: chat.title.clone(),
                    active: ix == self.active,
                    at: chat.created_at,
                }
            })
            .collect()
    }
}

/// Best fuzzy score across a command's label and keywords.
fn rank_command(spec: &CommandSpec, query: &str) -> Option<i32> {
    std::iter::once(spec.label)
        .chain(spec.keywords.iter().copied())
        .filter_map(|s| crate::palette_fuzzy::fuzzy_score(query, s))
        .max()
}

/// All palette entries for `query`, in display order: commands first, then
/// chats. A non-empty query keeps only fuzzy matches, best score first within
/// each group; an empty query lists everything in table/sidebar order.
pub(crate) fn build_entries(chats: &[ChatSnapshot], query: &str, running: usize) -> Vec<Entry> {
    let query = query.trim();
    let specs = crate::palette_commands::command_specs(running);
    let mut commands: Vec<(usize, i32)> = specs
        .iter()
        .enumerate()
        .filter_map(|(ix, spec)| if query.is_empty() { Some((ix, 0)) } else { rank_command(spec, query).map(|score| (ix, score)) })
        .collect();
    if !query.is_empty() {
        commands.sort_by_key(|(ix, score)| (std::cmp::Reverse(*score), *ix));
    }

    let mut matched: Vec<(usize, i32)> = chats
        .iter()
        .enumerate()
        .filter_map(|(ix, chat)| {
            if query.is_empty() {
                Some((ix, 0))
            } else {
                crate::palette_fuzzy::fuzzy_score(query, &chat.title).map(|score| (ix, score))
            }
        })
        .collect();
    if !query.is_empty() {
        matched.sort_by_key(|(ix, score)| (std::cmp::Reverse(*score), *ix));
    }

    commands
        .into_iter()
        .map(|(ix, _)| {
            let spec = &specs[ix];
            Entry::Command(CommandSpec {
                label: spec.label,
                icon: spec.icon,
                keywords: spec.keywords,
                effect: match &spec.effect {
                    Effect::Dispatch(a) => Effect::Dispatch(a.boxed_clone()),
                    Effect::Run(f) => Effect::Run(*f),
                },
            })
        })
        .chain(matched.into_iter().map(|(ix, _)| {
            let chat = &chats[ix];
            Entry::Chat(ChatSnapshot {
                id: chat.id,
                title: chat.title.clone(),
                active: chat.active,
                at: chat.at,
            })
        }))
        .collect()
}

/// The entry a confirmed `IndexPath` refers to. Section 0 is the commands
/// group, section 1 the chats group — `Command`'s section numbering counts
/// every group in the model, so these stay stable even when a group filters
/// to nothing.
pub(crate) fn entry_at(chats: &[ChatSnapshot], query: &str, path: IndexPath, running: usize) -> Option<Entry> {
    let mut command_row = 0usize;
    let mut chat_row = 0usize;
    build_entries(chats, query, running).into_iter().find(|entry| {
        let hit = match entry {
            Entry::Command(_) => path.section == 0 && path.row == command_row,
            Entry::Chat(_) => path.section == 1 && path.row == chat_row,
        };
        match entry {
            Entry::Command(_) => command_row += 1,
            Entry::Chat(_) => chat_row += 1,
        }
        hit
    })
}

/// The two `Command` groups for the current query — headings hide themselves
/// when their group is empty.
pub(crate) fn palette_groups(chats: &[ChatSnapshot], query: &str, running: usize) -> (CommandGroup, CommandGroup) {
    let mut commands = CommandGroup::new().label("Commands");
    let mut chat_group = CommandGroup::new().label("Chats");
    for entry in build_entries(chats, query, running) {
        match entry {
            Entry::Command(spec) => commands = commands.item(command_item(spec)),
            Entry::Chat(chat) => chat_group = chat_group.item(chat_item(chat)),
        }
    }
    (commands, chat_group)
}

fn command_item(spec: CommandSpec) -> CommandItem {
    let mut item = CommandItem::new().label(spec.label).icon(spec.icon).keywords(spec.keywords.iter().copied());
    if let Effect::Dispatch(action) = spec.effect {
        item = item.action(action);
    }
    item
}

/// A chat row: terminal icon + title + relative age, with a check on the
/// active chat. Custom content replaces the label/icon slot; the check still
/// renders at the trailing edge.
fn chat_item(chat: ChatSnapshot) -> CommandItem {
    let title = chat.title.clone();
    CommandItem::new().label(chat.title).checked(chat.active).child(move |_, cx| {
        h_flex()
            .flex_1()
            .gap_2()
            .items_center()
            .child(IconName::SquareTerminal)
            .child(title.clone())
            .child(div().flex_1())
            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(rel_time(chat.at)))
    })
}

/// "2h ago"-style age for chat rows.
pub(crate) fn rel_time(at: std::time::SystemTime) -> SharedString {
    let secs = at.elapsed().map(|d| d.as_secs()).unwrap_or(0);
    match secs {
        s if s < 60 => "just now".into(),
        s if s < 3_600 => format!("{}m ago", s / 60).into(),
        s if s < 86_400 => format!("{}h ago", s / 3_600).into(),
        s => format!("{}d ago", s / 86_400).into(),
    }
}

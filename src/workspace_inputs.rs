//! The input entities `Workspace::for_project` wires up — composer, search
//! fields, palettes, and their subscriptions. Split from `workspace_new.rs`
//! for the SLOC cap; every subscription here registers the Workspace as the
//! subscriber, so the closures' `this` is the workspace itself.

use crate::workspace::Workspace;
use gpui_kit::component::command::CommandState;
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::component::message_scroller::MessageScrollerState;
use gpui_kit::*;

/// The entities `for_project` builds before the `Workspace` struct literal —
/// grouped so the constructor stays under the file-size cap.
pub(crate) struct WorkspaceInputs {
    pub composer: Entity<TextareaState>,
    pub scroller: Entity<MessageScrollerState>,
    pub search: Entity<InputState>,
    pub palette: Entity<CommandState>,
    pub chat_search: Entity<InputState>,
    pub global_search: Entity<CommandState>,
    pub file_palette: Entity<CommandState>,
    pub task_input: Entity<InputState>,
    pub terminal_input: Entity<InputState>,
    pub terminal_find_input: Entity<InputState>,
    pub hotkey_input: Entity<InputState>,
    /// The General section's budget-cap field — Enter or blur commits.
    pub budget_cap_input: Entity<InputState>,
}

impl WorkspaceInputs {
    /// Create every input entity and wire its subscriptions. `global_hotkey`
    /// is the persisted chord shown in the settings field; `budget_cap` is
    /// the persisted global spend cap shown in the budget field.
    pub(crate) fn build(global_hotkey: &str, budget_cap: Option<f64>, window: &mut Window, cx: &mut Context<Workspace>) -> Self {
        let composer = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(1, 8)
                .submit_on_enter(true)
                .placeholder("Ask anything — @ to mention files, / for commands")
        });
        let scroller = cx.new(|cx| MessageScrollerState::new(0, cx));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search chats"));

        cx.subscribe_in(&search, window, |this, _s, event: &InputEvent, _window, cx| {
            if matches!(event, InputEvent::Change) {
                // Title filtering re-reads the input in render; the
                // message-body scan runs here (live chats now, disk after
                // a debounce — see `crate::global_search::sidebar_search`).
                this.schedule_sidebar_search(cx);
                cx.notify()
            }
        })
        .detach();
        cx.subscribe_in(&composer, window, |this, _composer, event: &InputEvent, window, cx| match event {
            InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
            // `set_value` suppresses Change, so this only fires on real edits.
            InputEvent::Change => {
                this.clear_recall();
                // Write the draft through to the chat so the debounced save
                // (and the switch/quit stashes) persist it. A queued-message
                // edit borrows the composer — its text isn't the draft.
                if let Some(chat) = this.chats.get_mut(this.active)
                    && !this.send_queue.editing_for(chat.id)
                {
                    chat.draft = this.composer.read(cx).value().to_string();
                    this.draft_save_ticks.get_or_insert(0);
                }
                cx.notify();
            },
            _ => {},
        })
        .detach();

        let palette = cx.new(|cx| CommandState::new(window, cx));
        let chat_search = cx.new(|cx| InputState::new(window, cx).placeholder("Search in chat"));
        cx.subscribe_in(&chat_search, window, |this, _s, event: &InputEvent, _window, cx| match event {
            InputEvent::Change => {
                this.search_match_ix = 0;
                let count = this.filtered_count(cx);
                this.scroller.update(cx, |s, cx| s.reset(count, cx));
                cx.notify();
            },
            InputEvent::PressEnter { shift, .. } => this.jump_to_match(*shift, cx),
            _ => {},
        })
        .detach();
        let global_search = cx.new(|cx| CommandState::new(window, cx));
        let file_palette = cx.new(|cx| CommandState::new(window, cx));
        let task_input = cx.new(|cx| InputState::new(window, cx).placeholder("New task…"));
        cx.subscribe_in(&task_input, window, |this, s, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                let prompt = s.read(cx).value().to_string();
                s.update(cx, |s, cx| s.set_value("", window, cx));
                this.spawn_task_agent(prompt, cx);
            }
        })
        .detach();
        let terminal_input = cx.new(|cx| InputState::new(window, cx).placeholder("Run a command…"));
        cx.subscribe_in(&terminal_input, window, |this, _s, event: &InputEvent, window, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.terminal_send(window, cx);
            }
        })
        .detach();
        let terminal_find_input = crate::views::terminal::find::new_term_find_input(window, cx);
        // Cmd+Q / QuitApp bypasses the window close gate — save drafts here.
        cx.on_app_quit(|this, cx| {
            this.chats[this.active].draft = this.composer.read(cx).value().to_string();
            this.save();
            async {}
        })
        .detach();
        let budget_cap_input = new_budget_cap_input(budget_cap, window, cx);
        let hotkey_input = new_hotkey_input(global_hotkey, window, cx);
        Self {
            composer,
            scroller,
            search,
            palette,
            chat_search,
            global_search,
            file_palette,
            task_input,
            terminal_input,
            terminal_find_input,
            budget_cap_input,
            hotkey_input,
        }
    }
}

/// The General section's global-hotkey field — Enter or blur commits the
/// chord (validation lives in `commit_global_hotkey`).
fn new_hotkey_input(chord: &str, window: &mut Window, cx: &mut Context<Workspace>) -> Entity<InputState> {
    let input = cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder(crate::app_setup::global_hotkey::DEFAULT_CHORD);
        input.set_value(chord.to_string(), window, cx);
        input
    });
    cx.subscribe_in(&input, window, |this, _s, event: &InputEvent, window, cx| {
        if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
            this.commit_global_hotkey(window, cx);
        }
    })
    .detach();
    input
}

/// The General section's budget-cap field — Enter or blur commits the
/// amount (parsing lives in `commit_budget_cap`); empty means no cap.
fn new_budget_cap_input(cap: Option<f64>, window: &mut Window, cx: &mut Context<Workspace>) -> Entity<InputState> {
    let input = cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder("e.g. 5.00 — empty = no cap");
        input.set_value(cap.map(|c| c.to_string()).unwrap_or_default(), window, cx);
        input
    });
    cx.subscribe_in(&input, window, |this, _s, event: &InputEvent, window, cx| {
        if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
            this.commit_budget_cap(window, cx);
        }
    })
    .detach();
    input
}

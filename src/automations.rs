//! Scheduled prompts — Codex "automations"-style. An `Automation` re-sends
//! a prompt into its chat on a fixed interval while the app is open: the
//! 1s workspace ticker (see `lifecycle::start_background`) calls
//! `fire_due_automations`, which routes the prompt through the same
//! chat-keyed send path a typed message takes, so the reply streams into
//! the transcript like any other turn.
//!
//! Automations persist per project as `<project>/automations.json` (see
//! `crate::persist::{save_automations, load_automations}`). A chat deleted
//! while its automation exists drops the automation on the next tick; a
//! chat mid-turn skips that occurrence — `next_run` always advances from
//! the moment it was handled, so a missed window (app closed, chat busy)
//! runs once, never N times.

use std::time::{Duration, SystemTime};

use gpui_kit::component::WindowExt;
use gpui_kit::component::input::Textarea;
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::send_queue::Queued;
use crate::workspace::Workspace;

/// The fixed intervals a scheduled prompt can run on — presets only, no
/// cron. `name` is the persisted form and the chip label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum AutomationInterval {
    #[serde(rename = "15m")]
    M15,
    #[serde(rename = "1h")]
    H1,
    #[serde(rename = "6h")]
    H6,
    #[serde(rename = "24h")]
    H24,
}

impl AutomationInterval {
    pub(crate) const ALL: [Self; 4] = [Self::M15, Self::H1, Self::H6, Self::H24];

    pub(crate) fn duration(self) -> Duration {
        match self {
            Self::M15 => Duration::from_secs(15 * 60),
            Self::H1 => Duration::from_secs(60 * 60),
            Self::H6 => Duration::from_secs(6 * 60 * 60),
            Self::H24 => Duration::from_secs(24 * 60 * 60),
        }
    }

    /// Chip label and persisted name — "15m" | "1h" | "6h" | "24h".
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::M15 => "15m",
            Self::H1 => "1h",
            Self::H6 => "6h",
            Self::H24 => "24h",
        }
    }
}

/// One scheduled prompt: `prompt` re-runs in chat `chat_id` every
/// `interval` while `enabled`. `next_run` is the wall-clock time of the
/// next fire; `last_run` records the last actual send (a skipped
/// occurrence doesn't count).
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct Automation {
    pub id: u64,
    pub chat_id: u64,
    pub prompt: String,
    pub interval: AutomationInterval,
    pub enabled: bool,
    pub next_run: SystemTime,
    #[serde(default)]
    pub last_run: Option<SystemTime>,
}

impl Automation {
    /// Whether this automation should fire at `now`.
    pub(crate) fn due(&self, now: SystemTime) -> bool {
        self.enabled && self.next_run <= now
    }

    /// Record a fire: `last_run` stamps the send and the next occurrence
    /// lands one interval out from now — a `next_run` far in the past
    /// (app closed, chat busy) still runs once, not once per missed slot.
    pub(crate) fn mark_fired(&mut self, now: SystemTime) {
        self.last_run = Some(now);
        self.next_run = now + self.interval.duration();
    }

    /// A due occurrence the chat couldn't take (mid-turn): skip it —
    /// `last_run` stays untouched and the next slot is one interval out.
    pub(crate) fn skip(&mut self, now: SystemTime) {
        self.next_run = now + self.interval.duration();
    }
}

impl Workspace {
    /// The ⋯ menu's "Schedule…" — a small dialog seeded with the chat's
    /// draft (the composer text for the active chat) and interval chips;
    /// OK creates the automation. Ephemeral chats never reach disk, so
    /// they can't be scheduled — the menu disables the item.
    pub fn open_schedule_dialog(&mut self, chat_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(chat) = self.chats.iter().find(|c| c.id == chat_id) else { return };
        if chat.ephemeral {
            return;
        }
        let seed = if self.chats.get(self.active).is_some_and(|c| c.id == chat_id) {
            self.composer.read(cx).value().to_string()
        } else {
            chat.draft.clone()
        };
        self.schedule_prompt_input.update(cx, |state, cx| state.set_value(seed, window, cx));
        self.schedule_interval = AutomationInterval::H1;
        let ws = cx.entity();
        let input = self.schedule_prompt_input.clone();
        window.open_dialog(cx, move |dialog, _window, cx| {
            let ws_ok = ws.clone();
            let chips = interval_chips(&ws, cx);
            dialog
                .title("Schedule prompt")
                .overlay_closable(true)
                .child(
                    div().flex().flex_col().gap_3().child(Textarea::new(&input).aria_label("Scheduled prompt")).child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(div().text_sm().text_color(cx.theme().muted_foreground).child("Every"))
                            .children(chips),
                    ),
                )
                .on_ok(move |_, _window, cx| {
                    ws_ok.update(cx, |this, cx| this.commit_schedule(chat_id, cx));
                    true
                })
        });
        self.schedule_prompt_input.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Dialog OK for "Schedule prompt" — empty text creates nothing.
    fn commit_schedule(&mut self, chat_id: u64, cx: &mut Context<Self>) {
        let prompt = self.schedule_prompt_input.read(cx).value().trim().to_string();
        if prompt.is_empty() {
            return;
        }
        let interval = self.schedule_interval;
        let automation = Automation {
            id: self.next_automation_id,
            chat_id,
            prompt,
            interval,
            enabled: true,
            next_run: SystemTime::now() + interval.duration(),
            last_run: None,
        };
        self.next_automation_id += 1;
        self.automations.push(automation);
        self.persist_automations();
        cx.notify();
    }

    /// The panel row's enable switch — re-enabling schedules the next run
    /// one interval out rather than firing a backlog of missed slots.
    pub(crate) fn toggle_automation(&mut self, id: u64, enabled: bool, cx: &mut Context<Self>) {
        let Some(a) = self.automations.iter_mut().find(|a| a.id == id) else { return };
        a.enabled = enabled;
        if enabled {
            a.next_run = SystemTime::now() + a.interval.duration();
        }
        self.persist_automations();
        cx.notify();
    }

    /// The panel row's delete button.
    pub(crate) fn delete_automation(&mut self, id: u64, cx: &mut Context<Self>) {
        let before = self.automations.len();
        self.automations.retain(|a| a.id != id);
        if self.automations.len() != before {
            self.persist_automations();
            cx.notify();
        }
    }

    /// One scheduler pass (the workspace's 1s tick): fire every due
    /// automation. A chat mid-turn skips the occurrence; a deleted chat
    /// drops its automation. Firing sends the prompt through the normal
    /// chat turn path — the reply streams into that chat's transcript.
    pub(crate) fn fire_due_automations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = SystemTime::now();
        let due: Vec<u64> = self.automations.iter().filter(|a| a.due(now)).map(|a| a.id).collect();
        if due.is_empty() {
            return;
        }
        for id in due {
            let Some(ix) = self.automations.iter().position(|a| a.id == id) else { continue };
            let (chat_id, prompt) = (self.automations[ix].chat_id, self.automations[ix].prompt.clone());
            let Some(chat_ix) = self.chat_index(chat_id) else {
                // The chat is gone — its automation goes with it.
                self.automations.remove(ix);
                continue;
            };
            if self.chats[chat_ix].running {
                self.automations[ix].skip(now);
                continue;
            }
            self.automations[ix].mark_fired(now);
            self.send_text_in(chat_id, Queued::new(prompt, Vec::new()), window, cx);
        }
        self.persist_automations();
    }

    /// Write `automations.json` — called after every mutation.
    pub(crate) fn persist_automations(&self) {
        crate::persist::save_automations(self.project.dir(), &self.automations);
    }

    /// Toggle the Scheduled panel; the open flag persists like the plan
    /// panel's (`Settings.scheduled_panel_open`).
    pub fn toggle_scheduled_panel(&mut self, cx: &mut Context<Self>) {
        self.scheduled_panel_open = !self.scheduled_panel_open;
        self.save_settings();
        cx.notify();
    }
}

/// The dialog's interval chips — one per preset, accent-bordered while
/// selected. Clicking writes `schedule_interval` and notifies so the
/// dialog re-renders with the new pick.
fn interval_chips(ws: &Entity<Workspace>, cx: &App) -> [impl IntoElement; 4] {
    let current = ws.read(cx).schedule_interval;
    AutomationInterval::ALL.map(|interval| {
        let selected = interval == current;
        let ws = ws.clone();
        div()
            .id(SharedString::from(format!("schedule-interval-{}", interval.label())))
            .test_support()
            .cursor_pointer()
            .px_2()
            .py_1()
            .rounded_md()
            .border_1()
            .text_sm()
            .border_color(if selected { cx.theme().accent } else { cx.theme().border })
            .when(selected, |d| d.text_color(cx.theme().accent))
            .child(interval.label())
            .on_click(move |_, _, cx| {
                ws.update(cx, |this, cx| {
                    this.schedule_interval = interval;
                    cx.notify();
                });
            })
    })
}

// Declared here, not in `main.rs` — the crate root is at the SLOC cap.
#[cfg(test)]
#[path = "automations_tests.rs"]
mod automations_tests;

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::menu::{DropdownMenu, PopupMenuItem};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::model::Chat;
use crate::send_queue::Queued;
use crate::workspace::Workspace;

pub fn slash_item(cmd: &str, ws: &Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let ws = ws.clone();
    let cmd_str = cmd.to_string();
    div()
        .id(SharedString::from(format!("slash-{cmd}")))
        .test_support()
        .cursor_pointer()
        .px_2()
        .py_1()
        .rounded_md()
        .text_sm()
        .hover(|d| d.bg(cx.theme().accent))
        .child(format!("/{cmd}"))
        .on_click(move |_, window, cx| {
            apply_slash(&ws, &cmd_str, window, cx);
        })
}

pub fn mention_item(file: &str, ws: &Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let ws = ws.clone();
    let path = file.to_string();
    div()
        .id(SharedString::from(format!("mention-{file}")))
        .test_support()
        .cursor_pointer()
        .px_2()
        .py_1()
        .rounded_md()
        .text_sm()
        .hover(|d| d.bg(cx.theme().accent))
        .child(format!("@{file}"))
        .on_click(move |_, window, cx| {
            apply_mention(&ws, &path, window, cx);
        })
}

pub fn attachment_chips(chat: &Chat, ws: &Entity<Workspace>, cx: &mut App) -> Vec<AnyElement> {
    chat.attachments
        .iter()
        .enumerate()
        .map(|(ix, path)| {
            let ws = ws.clone();
            let full = path.to_string();
            let name = std::path::Path::new(&full)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or(full.clone());
            div()
                .id(ix)
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .py_0p5()
                .rounded_md()
                .bg(cx.theme().secondary)
                .text_xs()
                .child(IconName::FileText)
                .child(
                    div()
                        .id(("reveal-attach", ix))
                        .cursor_pointer()
                        .child(name)
                        .on_click(move |_, _, cx| cx.reveal_path(std::path::Path::new(&full))),
                )
                .child(
                    div()
                        .id(("remove-attach", ix))
                        .cursor_pointer()
                        .text_color(cx.theme().muted_foreground)
                        .child(IconName::X)
                        .on_click(move |_, _, cx| {
                            ws.update(cx, |this, cx| this.remove_attachment(ix, cx));
                        }),
                )
                .into_any_element()
        })
        .collect()
}

fn apply_slash(ws: &Entity<Workspace>, cmd: &str, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| this.run_command(cmd, window, cx));
}
fn apply_mention(ws: &Entity<Workspace>, path: &str, window: &mut Window, cx: &mut App) {
    ws.update(cx, |this, cx| {
        this.composer.update(cx, |s, cx| {
            let cur = s.value().to_string();
            let before = cur.rsplit_once('@').map(|(b, _)| b).unwrap_or("");
            s.set_value(format!("{before}@{path} "), window, cx);
            s.focus(window, cx);
        });
        // `set_value` suppresses Change — nudge the workspace so the menu closes.
        cx.notify();
    });
}

pub fn apply_pick(ws: &Entity<Workspace>, set: fn(&mut Workspace, &'static str), opt: &'static str, cx: &mut App) {
    ws.update(cx, |this, cx| {
        set(this, opt);
        cx.notify();
    });
}

/// Owned inputs for `model_picker` — the composer builds this from
/// `&Workspace` so the returned element holds no borrow.
pub struct ModelPickerSpec {
    /// Selected instance id — empty when no instance exists.
    pub current_provider: String,
    pub current_model: SharedString,
    /// Enabled instances as (id, display name, effective model list).
    pub providers: Vec<(String, String, Vec<crate::model::ModelInfo>)>,
    pub ws: Entity<Workspace>,
}

/// The provider→model dropdown: one submenu per enabled instance listing
/// its effective catalog. Picking a model under another instance switches
/// the active backend too. An instance with no catalog shows an empty
/// submenu — there is no synthetic default entry.
pub fn model_picker(spec: ModelPickerSpec) -> impl IntoElement {
    let ModelPickerSpec { current_provider, current_model, providers, ws: ws_entity } = spec;
    let provider_label = providers
        .iter()
        .find(|(id, _, _)| *id == current_provider)
        .map_or_else(|| current_provider.clone(), |(_, name, _)| name.clone());
    let model_label = providers
        .iter()
        .find(|(id, _, _)| *id == current_provider)
        .and_then(|(_, _, opts)| opts.iter().find(|m| m.id == current_model))
        .map_or_else(|| current_model.to_string(), |m| m.label.to_string());
    let label = format!("{provider_label} · {model_label}");
    Button::new("model")
        .ghost()
        .label(label)
        .icon(IconName::ChevronsUpDown)
        .dropdown_menu(move |menu, window, cx| {
            providers.iter().fold(menu, |menu, (id, name, options)| {
                let item_ctx = ModelItemCtx {
                    ws: ws_entity.clone(),
                    current_provider: current_provider.clone(),
                    current_model: current_model.clone(),
                    provider: id.clone(),
                };
                let options = options.clone();
                let name = name.clone();
                menu.submenu(name, window, cx, move |sub, _w, _cx| {
                    options.iter().cloned().fold(sub, |sub, m| sub.item(model_menu_item(&item_ctx, m)))
                })
            })
        })
}

/// Everything one submenu item needs — bundled to keep the builder under
/// the nesting/arg-count lints.
struct ModelItemCtx {
    ws: Entity<Workspace>,
    current_provider: String,
    current_model: SharedString,
    /// The instance this submenu lists models for.
    provider: String,
}

/// One clickable model row: check when selected, click selects
/// instance+model on the workspace.
fn model_menu_item(ctx: &ModelItemCtx, m: crate::model::ModelInfo) -> PopupMenuItem {
    let checked = ctx.current_provider == ctx.provider && ctx.current_model.as_ref() == m.id.as_ref();
    let ws = ctx.ws.clone();
    let pid = ctx.provider.clone();
    let mid = m.id.clone();
    let pid2 = pid.clone();
    PopupMenuItem::element(move |_, cx| model_option(&pid2, &m, cx))
        .checked(checked)
        .on_click(move |_, _, cx| {
            ws.update(cx, |this, cx| {
                this.select_model(&pid, &mid, cx);
            });
        })
}

/// One model row inside a provider submenu — label plus the model's
/// one-line description, dimmed.
fn model_option(provider: &str, m: &crate::model::ModelInfo, cx: &App) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    div()
        .id(SharedString::from(format!("model-opt-{provider}-{}", m.id)))
        .test_support()
        .flex()
        .items_baseline()
        .gap_2()
        .child(m.label.clone())
        .when(!m.description.is_empty(), |d| d.child(div().text_xs().text_color(cx.theme().muted_foreground).child(m.description.clone())))
}

/// One queued-message row: dimmed text plus an ✕ that drops it.
pub fn queued_item(item: &Queued, ws: &Entity<Workspace>, cx: &mut App) -> impl IntoElement {
    let ws = ws.clone();
    let id = item.id;
    div()
        .id(SharedString::from(format!("queued-{}", item.id)))
        .test_support()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .whitespace_nowrap()
                .text_ellipsis()
                .child(item.text.clone()),
        )
        .child(
            div().id(SharedString::from(format!("dequeue-{}", item.id))).test_support().child(
                Button::new(SharedString::from(format!("dequeue-btn-{}", item.id)))
                    .ghost()
                    .xsmall()
                    .icon(IconName::X)
                    .on_click(move |_, _, cx| {
                        ws.update(cx, |this, cx| {
                            this.send_queue.remove(this.chats[this.active].id, id);
                            cx.notify();
                        });
                    }),
            ),
        )
}

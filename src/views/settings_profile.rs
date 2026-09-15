//! The Profile settings section: the signed-in provider accounts (read from
//! the workspace's `AuthBook` — no new auth flows live here), the app
//! version, the data directory with a Reveal-in-Finder button, and a
//! "Sign out all" action. When nothing is signed in the section collapses
//! to a signed-out state that links to Providers settings.

use gpui_kit::assets::IconName;
use gpui_kit::component::Sizable;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::prelude::*;
use gpui_kit::*;

use crate::auth::{self, AuthState};
use crate::views::settings_nav::Section;
use crate::views::settings_sections::{SettingsView, group_label};

/// One account row: the instance plus its resolved auth state. `env_keyed`
/// marks credentials that live in an environment variable — they can't be
/// signed out from here, so the row says where the credential comes from.
struct AccountRow {
    id: String,
    name: String,
    icon: IconName,
    state: AuthState,
    env_keyed: bool,
}

/// The credential-bearing instances with their current auth state — kinds
/// with no credentials at all (sim) don't get account rows.
fn account_rows(s: &SettingsView, cx: &App) -> Vec<AccountRow> {
    let ws = s.ws.read(cx);
    ws.providers
        .iter()
        .filter(|p| auth::env_key(p).is_some() || auth::can_sign_in(p.kind))
        .map(|p| AccountRow {
            id: p.id.clone(),
            name: p.name.clone(),
            icon: p.kind.info().icon,
            state: ws.auth_state(&p.id),
            env_keyed: auth::env_key(p).is_some(),
        })
        .collect()
}

/// The Profile content pane: accounts (or the signed-out state), then the
/// About block with version + data directory.
pub(crate) fn profile_section(s: &SettingsView, cx: &App) -> impl IntoElement {
    let rows = account_rows(s, cx);
    let any_signed_in = rows.iter().any(|r| matches!(r.state, AuthState::SignedIn(_)));
    // Only flow-based kinds can be signed out — env-keyed credentials are
    // the user's environment, not a session we can revoke.
    let any_signoutable = rows
        .iter()
        .any(|r| !r.env_keyed && matches!(r.state, AuthState::SignedIn(_) | AuthState::SigningIn(_) | AuthState::AwaitingCode(_)));

    div()
        .flex()
        .flex_col()
        .gap_3()
        .child(group_label("Accounts", cx))
        .child(if any_signed_in {
            div().flex().flex_col().gap_1().children(rows.iter().map(|r| account_row(r, cx))).into_any_element()
        } else {
            signed_out_state(s, cx).into_any_element()
        })
        .when(any_signoutable, |d| d.child(sign_out_all_button(s)))
        .child(group_label("About", cx))
        .child(info_row("profile-version", "Version", env!("CARGO_PKG_VERSION"), cx))
        .child(data_dir_row(cx))
}

/// One signed-in-capable provider: kind icon, instance name, and the auth
/// status line. The row's `aria_label` carries "name — status" so tests can
/// read it (snapshots don't capture rendered text).
fn account_row(row: &AccountRow, cx: &App) -> impl IntoElement {
    let status = match &row.state {
        AuthState::SignedIn(detail) if row.env_keyed => format!("Authenticated via {detail}"),
        AuthState::SignedIn(detail) if detail.is_empty() => "Authenticated".to_string(),
        AuthState::SignedIn(detail) => format!("Authenticated · {detail}"),
        other => other.detail_status(),
    };
    let status_color = match &row.state {
        AuthState::SignedIn(_) => cx.theme().success,
        AuthState::SignedOut => cx.theme().danger,
        _ => cx.theme().muted_foreground,
    };
    div()
        .id(SharedString::from(format!("profile-account-{}", row.id)))
        .test_support()
        .aria_label(format!("{} — {status}", row.name))
        .flex()
        .items_center()
        .gap_2()
        .text_xs()
        .child(row.icon)
        .child(div().text_sm().child(row.name.clone()))
        .child(div().flex_1())
        .child(div().text_color(status_color).child(status))
}

/// The no-accounts state: a note plus a link that jumps to the Providers
/// section, where sign-in actually happens.
fn signed_out_state(s: &SettingsView, cx: &App) -> impl IntoElement {
    let panel = s.panel.clone();
    div()
        .id("profile-signed-out")
        .test_support()
        .aria_label("No accounts signed in")
        .flex()
        .flex_col()
        .gap_2()
        .child(div().text_sm().text_color(cx.theme().muted_foreground).child("No accounts signed in"))
        .child(
            Button::new("profile-open-providers")
                .label("Open Providers settings")
                .icon(IconName::ArrowRight)
                .small()
                .ghost()
                .on_click(move |_, _, cx| {
                    panel.update(cx, |this, cx| {
                        this.section = Section::Providers;
                        cx.notify();
                    });
                }),
        )
}

/// Revokes every provider session that has a logout flow.
fn sign_out_all_button(s: &SettingsView) -> impl IntoElement {
    let ws = s.ws.clone();
    Button::new("profile-sign-out-all")
        .label("Sign out all")
        .icon(IconName::LogOut)
        .small()
        .outline()
        .on_click(move |_, _, cx| ws.update(cx, |this, cx| this.sign_out_all(cx)))
}

/// A label/value line in the About block — the value div carries the id +
/// `aria_label` so tests can read it.
fn info_row(id: &'static str, label: &'static str, value: impl Into<String>, cx: &App) -> impl IntoElement {
    let value = value.into();
    div().flex().items_center().gap_4().child(div().w(px(120.)).text_sm().child(label)).child(
        div()
            .id(SharedString::from(id))
            .test_support()
            .aria_label(value.clone())
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(value),
    )
}

/// The app data directory (`~/.rixl/rixlcode`) — settings, chats, auth and
/// model caches all live under it.
pub(crate) fn data_dir() -> std::path::PathBuf {
    crate::persist::dirs_home().join(".rixl/rixlcode")
}

/// The data-dir row with its Reveal button. The platform's `reveal_path` is
/// unimplemented under the test harness, so the button opens a `file://`
/// URL instead — on macOS that lands in Finder, and the test platform
/// records it as `opened_url`.
fn data_dir_row(cx: &App) -> impl IntoElement {
    let dir = data_dir();
    let shown = dir.display().to_string();
    let url = file_uri(&dir);
    div()
        .flex()
        .items_center()
        .gap_4()
        .child(div().w(px(120.)).text_sm().child("Data directory"))
        .child(
            div()
                .id("profile-data-dir")
                .test_support()
                .aria_label(shown.clone())
                .flex_1()
                .min_w_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(shown),
        )
        .child(
            Button::new("profile-reveal-data")
                .label("Reveal in Finder")
                .icon(IconName::FolderOpen)
                .small()
                .outline()
                .on_click(move |_, _, cx| cx.open_url(&url)),
        )
}

/// `file://` URI for a local path — percent-encodes everything outside
/// RFC 3986's unreserved + path-punctuation set. Same encoding as
/// `backend::acp_rpc`'s attachment URIs (that helper is module-private).
fn file_uri(path: &std::path::Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'/' | b'-' | b'_' | b'.' | b'~' => uri.push(*byte as char),
            b => uri.push_str(&format!("%{b:02X}")),
        }
    }
    uri
}

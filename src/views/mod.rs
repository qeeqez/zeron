mod agents_card;
mod agents_panel;
mod cards;
mod changes;
mod chat_view;
mod composer;
mod composer_helpers;
mod diff;
mod empty;
mod markdown;
mod message;
pub(crate) mod nav_row;
pub mod settings;
pub mod settings_appearance;
pub mod settings_nav;
pub mod settings_providers;
pub mod settings_sections;

mod sidebar;
mod sidebar_row;

pub use composer_helpers::{ModelPickerSpec, apply_pick, attachment_chips, mention_item, model_picker, queued_item, slash_item};
pub use empty::render_empty_state;
pub use message::render_message;

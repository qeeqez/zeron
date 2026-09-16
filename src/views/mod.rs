pub(crate) mod activity;
mod agents_card;
mod agents_panel;
mod approval;
mod cards;
mod changes;
mod changes_commits;
mod changes_git;
mod chat_view;
mod composer;
mod composer_helpers;
mod composer_voice;
mod diff;
mod diff_split;
mod empty;
pub(crate) mod explorer;
#[cfg(test)]
mod explorer_tests;
mod markdown;
#[cfg(test)]
mod markdown_tests;
mod message;
mod message_edit;
mod message_footer;
#[cfg(test)]
mod message_tests;
mod model_picker;
pub(crate) mod nav_row;
mod plan_panel;
mod project_switcher;
pub mod settings;
pub mod settings_appearance;
pub mod settings_default_model;
pub mod settings_general;
#[cfg(test)]
mod settings_general_tests;
pub mod settings_instructions;
#[cfg(test)]
mod settings_instructions_tests;
pub mod settings_mcp;
pub mod settings_mcp_ops;
pub mod settings_nav;
pub mod settings_profile;
#[cfg(test)]
mod settings_profile_tests;
pub mod settings_provider_detail;
pub mod settings_provider_env;
pub mod settings_provider_wizard;
mod settings_provider_wizard_steps;
pub mod settings_providers;
pub mod settings_sections;
pub mod settings_shortcuts;
#[cfg(test)]
mod settings_shortcuts_tests;
pub mod settings_voice;
pub mod shortcuts;

pub(crate) mod sidebar;
mod sidebar_row;
pub(crate) mod snapshots;
pub(crate) mod terminal;

pub use composer_helpers::{apply_pick, attachment_chips, mention_item, queued_item, slash_item, usage_indicator};
pub use empty::render_empty_state;
pub use message::render_message;
pub use model_picker::{EffortPickerSpec, ModelPickerSpec, PickerProvider, PickerSpec, effort_picker, model_picker, picker};

#[cfg(test)]
#[path = "../settings_mcp_tests.rs"]
mod settings_mcp_tests;

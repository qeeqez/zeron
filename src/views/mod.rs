pub(crate) mod activity;
mod agents_card;
mod agents_panel;
mod approval;
pub(crate) mod budget;
mod cards;
mod changes;
mod changes_commits;
mod changes_conflicts;
mod changes_git;
mod changes_git_branch;
mod changes_pr;
mod changes_stash;
mod chat_info;
mod chat_menu;
mod chat_view;
mod composer;
mod composer_helpers;
mod composer_voice;
/// Day separators in the transcript — lives at `src/date_separator.rs`
/// because `main.rs` is at the SLOC cap (same pattern as `explorer_git`).
#[path = "../date_separator.rs"]
pub(crate) mod date_separator;
mod diff;
mod diff_split;
mod empty;
pub(crate) mod explorer;
#[path = "../explorer_git.rs"]
pub(crate) mod explorer_git;
#[cfg(test)]
#[path = "../explorer_git_tests.rs"]
mod explorer_git_tests;
#[cfg(test)]
mod explorer_tests;
pub(crate) mod file_inspect;
pub(crate) mod global_search;
pub(crate) mod logs;
mod markdown;
#[cfg(test)]
mod markdown_tests;
mod message;
mod message_edit;
mod message_footer;
#[cfg(test)]
mod message_menu_tests;
#[cfg(test)]
mod message_tests;
mod model_picker;
pub(crate) mod nav_row;
mod plan_panel;
mod project_switcher;
pub(crate) mod rate_limit;
mod retry_menu;
mod saved_prompts;
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
pub mod settings_project;
#[cfg(test)]
mod settings_project_tests;
pub mod settings_provider_detail;
pub mod settings_provider_env;
pub mod settings_provider_test;
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
mod terminal_tabs;
#[cfg(test)]
mod tool_group_tests;
pub(crate) mod trust;
pub(crate) mod usage_dashboard;
mod usage_popover;

pub use composer_helpers::{apply_pick, attachment_chips, mention_item, queued_item, slash_item};
pub use empty::render_empty_state;
pub use message::render_message;
pub use model_picker::{EffortPickerSpec, ModelPickerSpec, PickerProvider, PickerSpec, effort_picker, model_picker, picker};
pub use saved_prompts::{SavedPromptsSpec, saved_prompts_popover};
pub use usage_popover::usage_popover;

#[cfg(test)]
#[path = "../onboarding_tests.rs"]
mod onboarding_tests;
#[cfg(test)]
#[path = "../settings_mcp_tests.rs"]
mod settings_mcp_tests;
#[cfg(test)]
#[path = "../starter_prompts_tests.rs"]
mod starter_prompts_tests;

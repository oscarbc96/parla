//! Parla application shell: globals, reload wiring, actions and the root view.

// The catalog is embedded from `i18n/*.ftl`; `t!` is defined here for every module.
i18n::namespace!("parla");

pub mod about;
pub mod actions;
mod app_state;
pub mod call_manager;
pub mod code_viewer;
pub mod crash;
pub mod delete_confirm;
pub mod dev_credentials;
pub mod diagnostics;
pub mod drafts;
pub mod error_text;
pub mod expiry_banner;
pub mod files;
pub mod fonts;
pub mod huddles;
pub mod idle_lock;
pub mod instance;
pub mod links;
pub mod local_schedules;
pub mod log;
pub mod login_item;
pub mod mcp_backend;
pub mod mcp_host;
pub mod mcp_status;
pub mod media;
pub mod media_viewer;
pub mod menus;
pub mod nav_history;
pub mod notifications;
pub mod notify_controls;
pub mod offline_banner;
#[cfg(feature = "fixture")]
pub mod perf;
pub mod platform;
pub mod plugins;
pub mod popout;
pub mod process_stats;
mod provider_calls;
pub mod providers;
pub mod quarantine;
pub mod reminders;
pub mod removal;
pub mod ringer;
pub mod root_view;
pub mod runtime;
pub mod send_timing;
pub mod session;
pub mod settings_window;
pub mod shortcuts;
pub mod sidebar_sections;
pub mod sign_in_window;
pub mod store_banner;
pub mod store_keys;
pub mod store_usage;
pub mod titlebar;
pub mod toast;
pub mod tour_state;
pub mod tray;
pub mod update_banner;
pub mod updates;
pub mod window_options;
pub mod window_state;
pub mod workspaces;
pub mod write_confirm_window;

/// The app shell catalog, for `i18n::gpui::init`.
pub fn catalog() -> &'static i18n::Catalog {
    &CATALOG
}

pub use app_state::{
    active_theme, handle_change, increase_contrast_changed, init, init_without_watcher,
};
pub use components::{Toast, ToastKind, Toasts};
pub use toast::ConfigBanner;

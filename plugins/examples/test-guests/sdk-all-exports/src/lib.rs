//! SDK test guest: every extension point through `export!`, so the SDK test
//! checks that the per-interface worlds merge into one component.

use parla_plugin_sdk::{Plugin, export};

struct All;

impl Plugin for All {}

export!(crate::All with [
    deactivation,
    migration,
    commands,
    slash_commands,
    message_actions,
    event_hooks,
    notification_filter,
    composer_transforms,
    renderer,
    views,
    settings_listener,
    mcp_tools,
    media_provider,
]);

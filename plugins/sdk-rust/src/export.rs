//! The [`Plugin`] trait and the [`export!`](crate::export) macro.
//!
//! Every extension point is a static method with a default: `activate`
//! succeeds, everything else returns [`Error::Unsupported`]. A plugin
//! overrides the ones its manifest declares and lists the same extension
//! points in `export!`, so the component exports exactly those interfaces.
//! The generated `Guest` traits are implemented for every `Plugin` here.

use crate::bindings::exports::parla::plugin::lifecycle;
use crate::ext;
use crate::types::{Context, Error, MessageRef, Version, ViewId};
use crate::view::Event as ViewEvent;
use crate::{hooks, media, notifications, renderer, slash, transforms};

/// A Parla plugin. Implement the methods for the extension points your
/// manifest declares; the rest keep their defaults.
#[allow(unused_variables)]
pub trait Plugin {
    /// `lifecycle.activate`. Deadline 1 s.
    fn activate(reason: lifecycle::ActivateReason) -> Result<(), Error> {
        Ok(())
    }
    /// `deactivation.deactivate`. Deadline 200 ms.
    fn deactivate() -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    /// `migration.migrate`. Deadline 5 s.
    fn migrate(previous: Version) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    /// `commands.run-command`.
    fn run_command(id: String, context: Context) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    /// `slash-commands.run-slash`.
    fn run_slash(
        view: ViewId,
        name: String,
        args: String,
        context: Context,
    ) -> Result<slash::SlashResult, Error> {
        Err(Error::Unsupported)
    }
    /// `message-actions.message-action`.
    fn message_action(id: String, message: MessageRef, context: Context) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    /// `event-hooks.on-event`.
    fn on_event(event: hooks::Event) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    /// `notification-filter.notification-filter`.
    fn notification_filter(
        notification: notifications::Notification,
    ) -> Result<notifications::Decision, Error> {
        Err(Error::Unsupported)
    }
    /// `composer-transforms.composer-transform`.
    fn composer_transform(id: String, draft: String) -> Result<transforms::TransformResult, Error> {
        Err(Error::Unsupported)
    }
    /// `renderer.render`.
    fn render(
        view: ViewId,
        id: String,
        url: String,
        context: Context,
    ) -> Result<renderer::Rendered, Error> {
        Err(Error::Unsupported)
    }
    /// `views.view-event`.
    fn view_event(view: ViewId, event: ViewEvent) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    /// `settings-listener.settings-changed`.
    fn settings_changed(keys: Vec<String>) -> Result<(), Error> {
        Err(Error::Unsupported)
    }
    /// `mcp-tools.mcp-call`.
    fn mcp_call(tool: String, json: String) -> Result<String, Error> {
        Err(Error::Unsupported)
    }
    /// `media-provider.media-search`.
    fn media_search(
        id: String,
        query: String,
        cursor: Option<String>,
        limit: u32,
        max_rating: media::ContentRating,
    ) -> Result<media::MediaPage, Error> {
        Err(Error::Unsupported)
    }
    /// `media-provider.media-trending`.
    fn media_trending(
        id: String,
        cursor: Option<String>,
        limit: u32,
        max_rating: media::ContentRating,
    ) -> Result<media::MediaPage, Error> {
        Err(Error::Unsupported)
    }
    /// `media-provider.media-resolve`.
    fn media_resolve(id: String, item_id: String) -> Result<media::MediaItem, Error> {
        Err(Error::Unsupported)
    }
}

impl<T: Plugin> lifecycle::Guest for T {
    fn activate(reason: lifecycle::ActivateReason) -> Result<(), Error> {
        <T as Plugin>::activate(reason)
    }
}

impl<T: Plugin> ext::deactivation::exports::parla::plugin::deactivation::Guest for T {
    fn deactivate() -> Result<(), Error> {
        <T as Plugin>::deactivate()
    }
}

impl<T: Plugin> ext::migration::exports::parla::plugin::migration::Guest for T {
    fn migrate(previous: Version) -> Result<(), Error> {
        <T as Plugin>::migrate(previous)
    }
}

impl<T: Plugin> ext::commands::exports::parla::plugin::commands::Guest for T {
    fn run_command(id: String, context: Context) -> Result<(), Error> {
        <T as Plugin>::run_command(id, context)
    }
}

impl<T: Plugin> slash::Guest for T {
    fn run_slash(
        view: ViewId,
        name: String,
        args: String,
        context: Context,
    ) -> Result<slash::SlashResult, Error> {
        <T as Plugin>::run_slash(view, name, args, context)
    }
}

impl<T: Plugin> ext::message_actions::exports::parla::plugin::message_actions::Guest for T {
    fn message_action(id: String, message: MessageRef, context: Context) -> Result<(), Error> {
        <T as Plugin>::message_action(id, message, context)
    }
}

impl<T: Plugin> hooks::Guest for T {
    fn on_event(event: hooks::Event) -> Result<(), Error> {
        <T as Plugin>::on_event(event)
    }
}

impl<T: Plugin> notifications::Guest for T {
    fn notification_filter(
        notification: notifications::Notification,
    ) -> Result<notifications::Decision, Error> {
        <T as Plugin>::notification_filter(notification)
    }
}

impl<T: Plugin> transforms::Guest for T {
    fn composer_transform(id: String, draft: String) -> Result<transforms::TransformResult, Error> {
        <T as Plugin>::composer_transform(id, draft)
    }
}

impl<T: Plugin> renderer::Guest for T {
    fn render(
        view: ViewId,
        id: String,
        url: String,
        context: Context,
    ) -> Result<renderer::Rendered, Error> {
        <T as Plugin>::render(view, id, url, context)
    }
}

impl<T: Plugin> ext::views::exports::parla::plugin::views::Guest for T {
    fn view_event(view: ViewId, event: ViewEvent) -> Result<(), Error> {
        <T as Plugin>::view_event(view, event)
    }
}

impl<T: Plugin> ext::settings_listener::exports::parla::plugin::settings_listener::Guest for T {
    fn settings_changed(keys: Vec<String>) -> Result<(), Error> {
        <T as Plugin>::settings_changed(keys)
    }
}

impl<T: Plugin> ext::mcp_tools::exports::parla::plugin::mcp_tools::Guest for T {
    fn mcp_call(tool: String, json: String) -> Result<String, Error> {
        <T as Plugin>::mcp_call(tool, json)
    }
}

impl<T: Plugin> media::Guest for T {
    fn media_search(
        id: String,
        query: String,
        cursor: Option<String>,
        limit: u32,
        max_rating: media::ContentRating,
    ) -> Result<media::MediaPage, Error> {
        <T as Plugin>::media_search(id, query, cursor, limit, max_rating)
    }
    fn media_trending(
        id: String,
        cursor: Option<String>,
        limit: u32,
        max_rating: media::ContentRating,
    ) -> Result<media::MediaPage, Error> {
        <T as Plugin>::media_trending(id, cursor, limit, max_rating)
    }
    fn media_resolve(id: String, item_id: String) -> Result<media::MediaItem, Error> {
        <T as Plugin>::media_resolve(id, item_id)
    }
}

/// Exports a [`Plugin`] as a component: `lifecycle` always, plus each listed
/// extension point, which must match the manifest's contributions (design
/// §10). The names are the interface names in snake case: `deactivation`,
/// `migration`, `commands`, `slash_commands`, `message_actions`,
/// `event_hooks`, `notification_filter`, `composer_transforms`, `renderer`,
/// `views`, `settings_listener`, `mcp_tools`, `media_provider`.
///
/// It expands only for `wasm32`: a native build (tests with
/// [`FakeHost`](crate::testing::FakeHost)) calls the `Plugin` methods
/// directly and has no component exports.
///
/// ```ignore
/// parla_plugin_sdk::export!(MyPlugin);                          // lifecycle only
/// parla_plugin_sdk::export!(MyPlugin with [commands, views]);  // plus two
/// parla_plugin_sdk::export!(crate::plugin::Mine with [renderer]); // a path
/// ```
#[macro_export]
macro_rules! export {
    ($($seg:ident)::+) => {
        $crate::export!($($seg)::+ with []);
    };
    ($($seg:ident)::+ with [$($ext:ident),* $(,)?]) => {
        // wit-bindgen's export macros take an identifier, so a path goes
        // through a local alias.
        #[cfg(target_arch = "wasm32")]
        const _: () = {
            type ParlaPluginExport = $($seg)::+;
            $crate::bindings::export_base!(ParlaPluginExport with_types_in $crate::bindings);
            $( $crate::ext::$ext::export!(ParlaPluginExport with_types_in $crate::ext::$ext); )*
        };
    };
}

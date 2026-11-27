//! Rust guest SDK for Parla plugins.
//!
//! A plugin implements [`Plugin`] and calls [`export!`] with the extension
//! points its manifest declares:
//!
//! ```ignore
//! use parla_plugin_sdk::{Error, Plugin, export, types::Context};
//!
//! struct Hello;
//! impl Plugin for Hello {
//!     fn run_command(_id: String, _cx: Context) -> Result<(), Error> {
//!         parla_plugin_sdk::host::ui::toast("Hello", parla_plugin_sdk::host::ui::ToastKind::Info)
//!     }
//! }
//! export!(Hello with [commands]);
//! ```
//!
//! The component then exports `lifecycle` plus exactly the listed
//! interfaces, so it matches its manifest.

#[cfg(all(not(target_arch = "wasm32"), not(feature = "testing")))]
compile_error!(
    "parla_plugin_sdk builds natively only for tests: build the plugin with \
     `cargo build --target wasm32-wasip2`, or test it with the SDK's `testing` \
     feature (a dev-dependency with `features = [\"testing\"]`)."
);

pub mod export;
pub mod host;
mod state;
#[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
pub mod testing;
pub mod view;

pub use bindings::exports::parla::plugin::lifecycle::ActivateReason;
pub use bindings::parla::plugin::types;
pub use export::Plugin;
pub use state::State;
pub use types::Error;

/// `composer-transforms` types: [`transforms::TransformResult`].
pub use ext::composer_transforms::exports::parla::plugin::composer_transforms as transforms;
/// `event-hooks` types: [`hooks::Event`] and its records.
pub use ext::event_hooks::exports::parla::plugin::event_hooks as hooks;
/// `media-provider` types: [`media::MediaItem`] and [`media::MediaPage`].
pub use ext::media_provider::exports::parla::plugin::media_provider as media;
/// `notification-filter` types: [`notifications::Notification`] and
/// [`notifications::Decision`].
pub use ext::notification_filter::exports::parla::plugin::notification_filter as notifications;
/// `renderer` types: [`renderer::Rendered`].
pub use ext::renderer::exports::parla::plugin::renderer;
/// `slash-commands` types: [`slash::SlashResult`].
pub use ext::slash_commands::exports::parla::plugin::slash_commands as slash;

#[doc(hidden)]
pub mod bindings {
    //! Bindings for the base world `parla:plugin/plugin`: every host import,
    //! the shared types and the `lifecycle` export. With `pub_export_macro`
    //! wit-bindgen describes the exports only inside the export macro, so a
    //! plugin's component exports what its `export!` call lists.
    wit_bindgen::generate!({
        world: "parla:plugin/plugin",
        path: "../../crates/parla_plugin_api/wit",
        pub_export_macro: true,
        default_bindings_module: "parla_plugin_sdk::bindings",
        export_macro_name: "export_base",
        generate_unused_types: true,
        additional_derives: [PartialEq],
    });
}

#[doc(hidden)]
pub mod ext {
    //! One module per optional extension point, each generated from a world
    //! that exports only that interface. `export!` invokes the export macro
    //! of each listed module, so wit-component merges exactly those worlds
    //! into the component. Shared types come from [`crate::bindings`].

    pub mod deactivation {
        wit_bindgen::generate!({
            path: "../../crates/parla_plugin_api/wit",
            inline: "package parla:sdk; world ext-deactivation { export parla:plugin/deactivation@0.1.0; }",
            world: "parla:sdk/ext-deactivation",
            pub_export_macro: true,
            additional_derives: [PartialEq],
            default_bindings_module: "parla_plugin_sdk::ext::deactivation",
            with: {
                "parla:plugin/types@0.1.0": crate::bindings::parla::plugin::types,
            },
        });
    }

    pub mod migration {
        wit_bindgen::generate!({
            path: "../../crates/parla_plugin_api/wit",
            inline: "package parla:sdk; world ext-migration { export parla:plugin/migration@0.1.0; }",
            world: "parla:sdk/ext-migration",
            pub_export_macro: true,
            additional_derives: [PartialEq],
            default_bindings_module: "parla_plugin_sdk::ext::migration",
            with: {
                "parla:plugin/types@0.1.0": crate::bindings::parla::plugin::types,
            },
        });
    }

    pub mod commands {
        wit_bindgen::generate!({
            path: "../../crates/parla_plugin_api/wit",
            inline: "package parla:sdk; world ext-commands { export parla:plugin/commands@0.1.0; }",
            world: "parla:sdk/ext-commands",
            pub_export_macro: true,
            additional_derives: [PartialEq],
            default_bindings_module: "parla_plugin_sdk::ext::commands",
            with: {
                "parla:plugin/types@0.1.0": crate::bindings::parla::plugin::types,
            },
        });
    }

    pub mod slash_commands {
        wit_bindgen::generate!({
            path: "../../crates/parla_plugin_api/wit",
            inline: "package parla:sdk; world ext-slash-commands { export parla:plugin/slash-commands@0.1.0; }",
            world: "parla:sdk/ext-slash-commands",
            pub_export_macro: true,
            additional_derives: [PartialEq],
            default_bindings_module: "parla_plugin_sdk::ext::slash_commands",
            with: {
                "parla:plugin/types@0.1.0": crate::bindings::parla::plugin::types,
                "parla:plugin/view@0.1.0": crate::bindings::parla::plugin::view,
            },
        });
    }

    pub mod message_actions {
        wit_bindgen::generate!({
            path: "../../crates/parla_plugin_api/wit",
            inline: "package parla:sdk; world ext-message-actions { export parla:plugin/message-actions@0.1.0; }",
            world: "parla:sdk/ext-message-actions",
            pub_export_macro: true,
            additional_derives: [PartialEq],
            default_bindings_module: "parla_plugin_sdk::ext::message_actions",
            with: {
                "parla:plugin/types@0.1.0": crate::bindings::parla::plugin::types,
            },
        });
    }

    pub mod event_hooks {
        wit_bindgen::generate!({
            path: "../../crates/parla_plugin_api/wit",
            inline: "package parla:sdk; world ext-event-hooks { export parla:plugin/event-hooks@0.1.0; }",
            world: "parla:sdk/ext-event-hooks",
            pub_export_macro: true,
            additional_derives: [PartialEq],
            default_bindings_module: "parla_plugin_sdk::ext::event_hooks",
            with: {
                "parla:plugin/types@0.1.0": crate::bindings::parla::plugin::types,
            },
        });
    }

    pub mod notification_filter {
        wit_bindgen::generate!({
            path: "../../crates/parla_plugin_api/wit",
            inline: "package parla:sdk; world ext-notification-filter { export parla:plugin/notification-filter@0.1.0; }",
            world: "parla:sdk/ext-notification-filter",
            pub_export_macro: true,
            additional_derives: [PartialEq],
            default_bindings_module: "parla_plugin_sdk::ext::notification_filter",
            with: {
                "parla:plugin/types@0.1.0": crate::bindings::parla::plugin::types,
            },
        });
    }

    pub mod composer_transforms {
        wit_bindgen::generate!({
            path: "../../crates/parla_plugin_api/wit",
            inline: "package parla:sdk; world ext-composer-transforms { export parla:plugin/composer-transforms@0.1.0; }",
            world: "parla:sdk/ext-composer-transforms",
            pub_export_macro: true,
            additional_derives: [PartialEq],
            default_bindings_module: "parla_plugin_sdk::ext::composer_transforms",
            with: {
                "parla:plugin/types@0.1.0": crate::bindings::parla::plugin::types,
            },
        });
    }

    pub mod renderer {
        wit_bindgen::generate!({
            path: "../../crates/parla_plugin_api/wit",
            inline: "package parla:sdk; world ext-renderer { export parla:plugin/renderer@0.1.0; }",
            world: "parla:sdk/ext-renderer",
            pub_export_macro: true,
            additional_derives: [PartialEq],
            default_bindings_module: "parla_plugin_sdk::ext::renderer",
            with: {
                "parla:plugin/types@0.1.0": crate::bindings::parla::plugin::types,
                "parla:plugin/view@0.1.0": crate::bindings::parla::plugin::view,
            },
        });
    }

    pub mod views {
        wit_bindgen::generate!({
            path: "../../crates/parla_plugin_api/wit",
            inline: "package parla:sdk; world ext-views { export parla:plugin/views@0.1.0; }",
            world: "parla:sdk/ext-views",
            pub_export_macro: true,
            additional_derives: [PartialEq],
            default_bindings_module: "parla_plugin_sdk::ext::views",
            with: {
                "parla:plugin/types@0.1.0": crate::bindings::parla::plugin::types,
                "parla:plugin/view@0.1.0": crate::bindings::parla::plugin::view,
            },
        });
    }

    pub mod settings_listener {
        wit_bindgen::generate!({
            path: "../../crates/parla_plugin_api/wit",
            inline: "package parla:sdk; world ext-settings-listener { export parla:plugin/settings-listener@0.1.0; }",
            world: "parla:sdk/ext-settings-listener",
            pub_export_macro: true,
            additional_derives: [PartialEq],
            default_bindings_module: "parla_plugin_sdk::ext::settings_listener",
            with: {
                "parla:plugin/types@0.1.0": crate::bindings::parla::plugin::types,
            },
        });
    }

    pub mod mcp_tools {
        wit_bindgen::generate!({
            path: "../../crates/parla_plugin_api/wit",
            inline: "package parla:sdk; world ext-mcp-tools { export parla:plugin/mcp-tools@0.1.0; }",
            world: "parla:sdk/ext-mcp-tools",
            pub_export_macro: true,
            additional_derives: [PartialEq],
            default_bindings_module: "parla_plugin_sdk::ext::mcp_tools",
            with: {
                "parla:plugin/types@0.1.0": crate::bindings::parla::plugin::types,
            },
        });
    }

    pub mod media_provider {
        wit_bindgen::generate!({
            path: "../../crates/parla_plugin_api/wit",
            inline: "package parla:sdk; world ext-media-provider { export parla:plugin/media-provider@0.1.0; }",
            world: "parla:sdk/ext-media-provider",
            pub_export_macro: true,
            additional_derives: [PartialEq],
            default_bindings_module: "parla_plugin_sdk::ext::media_provider",
            with: {
                "parla:plugin/types@0.1.0": crate::bindings::parla::plugin::types,
            },
        });
    }
}

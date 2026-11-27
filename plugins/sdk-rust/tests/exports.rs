//! The `Plugin` trait: every extension point but `activate`
//! defaults to `unsupported`, and the generated export traits forward to it.

use parla_plugin_sdk::types::{Context, MessageRef, Version};
use parla_plugin_sdk::view::{Event, ViewTree};
use parla_plugin_sdk::{ActivateReason, Error, Plugin};

struct Empty;
impl Plugin for Empty {}

fn ctx() -> Context {
    Context {
        workspace: None,
        conversation: None,
        thread: None,
        message: None,
    }
}

fn unsupported<T: std::fmt::Debug>(r: Result<T, Error>) {
    assert!(
        matches!(r, Err(Error::Unsupported)),
        "expected unsupported, got {r:?}"
    );
}

#[test]
fn default_exports_return_unsupported() {
    assert!(Empty::activate(ActivateReason::Install).is_ok());
    unsupported(Empty::deactivate());
    unsupported(Empty::migrate(Version {
        major: 0,
        minor: 1,
        patch: 0,
    }));
    unsupported(Empty::run_command("x".into(), ctx()));
    unsupported(Empty::run_slash(
        "v".into(),
        "/x".into(),
        String::new(),
        ctx(),
    ));
    let msg = MessageRef {
        conversation: "c".into(),
        message: "m".into(),
    };
    unsupported(Empty::message_action("a".into(), msg, ctx()));
    let ev =
        parla_plugin_sdk::hooks::Event::ChannelOpen(parla_plugin_sdk::hooks::ChannelOpenEvent {
            workspace: "w".into(),
            conversation: "c".into(),
        });
    unsupported(Empty::on_event(ev));
    let note = parla_plugin_sdk::notifications::Notification {
        workspace: "w".into(),
        conversation: "c".into(),
        message: None,
        title: "t".into(),
        body: "b".into(),
        sound: false,
    };
    unsupported(Empty::notification_filter(note));
    unsupported(Empty::composer_transform("t".into(), "draft".into()));
    unsupported(Empty::render(
        "v".into(),
        "r".into(),
        "https://example.com".into(),
        ctx(),
    ));
    unsupported(Empty::view_event("v".into(), Event::Refresh));
    unsupported(Empty::settings_changed(vec![]));
    unsupported(Empty::mcp_call("tool".into(), "{}".into()));
    unsupported(Empty::media_search("m".into(), "cats".into(), None, 10));
    unsupported(Empty::media_trending("m".into(), None, 10));
    unsupported(Empty::media_resolve("m".into(), "1".into()));
    let _unused: Option<ViewTree> = None;
}

#[test]
fn generated_export_traits_forward_to_plugin() {
    use parla_plugin_sdk::ext::commands::exports::parla::plugin::commands::Guest as Commands;
    use parla_plugin_sdk::ext::views::exports::parla::plugin::views::Guest as Views;
    unsupported(<Empty as Commands>::run_command("x".into(), ctx()));
    unsupported(<Empty as Views>::view_event("v".into(), Event::Closed));
}

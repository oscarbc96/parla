//! Typed host calls, one module per WIT interface. Each function
//! returns `Result<T, Error>`; its doc in the WIT names the capability or
//! workspace area the grant must hold.
//!
//! In a wasm build these call the component imports. In a native build with
//! the `testing` feature they call the [`FakeHost`](crate::testing::FakeHost)
//! installed on the current thread, so plugin code is tested unchanged.

/// Declares wrappers that forward to the wasm import (through
/// `lower::<iface>`, which adapts borrowed cursors and arguments) or, under
/// `testing` on a native target, to `crate::testing::imp::<iface>`.
macro_rules! wrap {
    ($iface:ident { $( fn $name:ident($($arg:ident: $ty:ty),* $(,)?) -> $ret:ty; )* }) => {
        $(
            #[doc = concat!("`", stringify!($iface), ".", stringify!($name), "` (see the WIT for the grant it needs).")]
            pub fn $name($($arg: $ty),*) -> Result<$ret, Error> {
                #[cfg(all(feature = "testing", not(target_arch = "wasm32")))]
                {
                    crate::testing::imp::$iface::$name($($arg),*)
                }
                #[cfg(not(all(feature = "testing", not(target_arch = "wasm32"))))]
                {
                    crate::host::lower::$iface::$name($($arg),*)
                }
            }
        )*
    };
}

pub use crate::bindings::parla::plugin::host::LogLevel;
use crate::types::{AppInfo, Error, Timestamp, Version};

// The `host` interface (always available): API version, app info, log,
// clock, the plugin's strings. Its functions sit at this module's root.
wrap!(host {
    fn api_version() -> Version;
    fn get_app_info() -> AppInfo;
    fn log(level: LogLevel, message: &str) -> ();
    fn now() -> Timestamp;
    fn t(key: &str, args: &[(&str, &str)]) -> String;
});

/// Reads and writes in the user's workspaces.
pub mod workspace {
    use crate::types::*;
    wrap!(workspace {
        fn list_workspaces() -> Vec<Workspace>;
        fn get_context() -> Context;
        fn me(ws: &str) -> User;
        fn list_conversations(ws: &str, filter: &ConversationFilter, cursor: Option<&str>, limit: u32) -> ConversationPage;
        fn get_conversation(ws: &str, conversation: &str) -> Conversation;
        fn history(ws: &str, conversation: &str, cursor: Option<&str>, limit: u32) -> MessagePage;
        fn replies(ws: &str, conversation: &str, thread: &str, cursor: Option<&str>, limit: u32) -> MessagePage;
        fn get_message(ws: &str, message: &MessageRef) -> Message;
        fn permalink(ws: &str, message: &MessageRef) -> String;
        fn get_user(ws: &str, user: &str) -> User;
        fn find_users(ws: &str, query: &str, limit: u32) -> Vec<User>;
        fn get_presence(ws: &str, user: &str) -> Presence;
        fn search_messages(ws: &str, query: &str, cursor: Option<&str>, limit: u32) -> MessagePage;
        fn search_files(ws: &str, query: &str, cursor: Option<&str>, limit: u32) -> FilePage;
        fn file_info(ws: &str, file: &str) -> File;
        fn file_bytes(ws: &str, file: &str, offset: u64, len: u32) -> Vec<u8>;
        fn list_reminders(ws: &str, cursor: Option<&str>, limit: u32) -> ReminderPage;
        fn list_scheduled(ws: &str, cursor: Option<&str>, limit: u32) -> ScheduledPage;
        fn list_saved(ws: &str, cursor: Option<&str>, limit: u32) -> MessageRefPage;
        fn list_pins(ws: &str, conversation: &str, cursor: Option<&str>, limit: u32) -> MessageRefPage;
        fn my_status(ws: &str) -> Status;
        fn send(ws: &str, conversation: &str, content: &Content) -> MessageRef;
        fn reply(ws: &str, conversation: &str, thread: &str, content: &Content, also_to_channel: bool) -> MessageRef;
        fn edit(ws: &str, message: &MessageRef, content: &Content) -> ();
        fn delete(ws: &str, message: &MessageRef) -> ();
        fn react(ws: &str, message: &MessageRef, emoji: &str) -> ();
        fn unreact(ws: &str, message: &MessageRef, emoji: &str) -> ();
        fn add_reminder(ws: &str, text: &str, at: Timestamp) -> ReminderId;
        fn complete_reminder(ws: &str, reminder: &str) -> ();
        fn delete_reminder(ws: &str, reminder: &str) -> ();
        fn schedule(ws: &str, conversation: &str, content: &Content, at: Timestamp) -> ScheduledId;
        fn cancel_scheduled(ws: &str, scheduled: &str) -> ();
        fn upload(ws: &str, conversation: &str, name: &str, bytes: &[u8], comment: Option<&str>) -> File;
        fn set_status(ws: &str, status: &Status) -> ();
        fn set_presence(ws: &str, presence: Presence) -> ();
        fn snooze(ws: &str, minutes: u32) -> ();
        fn end_snooze(ws: &str) -> ();
        fn set_saved(ws: &str, message: &MessageRef, saved: bool) -> ();
        fn set_pinned(ws: &str, message: &MessageRef, pinned: bool) -> ();
        fn mark_read(ws: &str, conversation: &str, upto: &str) -> ();
    });
}

/// Outbound HTTPS to granted hosts.
pub mod http {
    pub use crate::bindings::parla::plugin::http::{Method, Request, Response};
    use crate::types::Error;
    wrap!(http {
        fn fetch(request: &Request) -> Response;
    });
}

/// The plugin's own key-value store.
pub mod storage {
    pub use crate::bindings::parla::plugin::storage::Scope;
    use crate::types::{Error, KeyPage};
    wrap!(storage {
        fn get(scope: &Scope, key: &str) -> Option<Vec<u8>>;
        fn set(scope: &Scope, key: &str, value: &[u8]) -> ();
        fn delete(scope: &Scope, key: &str) -> ();
        fn keys(scope: &Scope, prefix: &str, cursor: Option<&str>, limit: u32) -> KeyPage;
        fn clear(scope: &Scope) -> ();
    });
}

/// The plugin's own secrets in the system keychain.
pub mod secrets {
    use crate::types::Error;
    wrap!(secrets {
        fn secret_get(key: &str) -> Option<String>;
        fn secret_set(key: &str, value: &str) -> ();
        fn secret_delete(key: &str) -> ();
    });
}

/// The plugin's own settings, declared in its manifest.
pub mod settings {
    pub use crate::bindings::parla::plugin::settings::SettingValue;
    use crate::types::Error;
    wrap!(settings {
        fn setting(key: &str) -> Option<SettingValue>;
    });
}

/// Panels, windows, toasts and navigation.
pub mod ui {
    pub use crate::bindings::parla::plugin::ui::{NotifyTarget, ToastKind, WindowSpec};
    use crate::types::{Error, ViewId};
    use crate::view::ViewTree;
    wrap!(ui {
        fn open_panel(panel_id: &str, tree: &ViewTree) -> ViewId;
        fn open_window(spec: &WindowSpec, tree: &ViewTree) -> ViewId;
        fn update(view: &str, tree: &ViewTree) -> ();
        fn patch(view: &str, node: &str, tree: &ViewTree) -> ();
        fn close(view: &str) -> ();
        fn refresh_after(view: &str, seconds: u32) -> ();
        fn toast(text: &str, kind: ToastKind) -> ();
        fn notify(title: &str, body: &str, target: &NotifyTarget) -> ();
        fn copy(text: &str) -> ();
        fn open_url(url: &str) -> ();
        fn open_conversation(ws: &str, conversation: &str, message: Option<&str>) -> ();
    });
}

/// The composer the plugin was invoked from.
pub mod composer {
    use crate::types::Error;
    wrap!(composer {
        fn draft() -> String;
        fn replace_draft(text: &str) -> ();
        fn insert(text: &str) -> ();
    });
}

/// Local programs with fixed commands (capability `exec`).
pub mod exec {
    pub use crate::bindings::parla::plugin::exec::ExecOutput;
    use crate::types::Error;
    wrap!(exec {
        fn run(program: &str, args: &[String]) -> ExecOutput;
    });
}

/// Adapters from the SDK's signatures to the generated imports: cursors as
/// `Option<&str>` and `t` arguments as `&str` pairs. Everything else is the
/// import itself.
#[cfg(not(all(feature = "testing", not(target_arch = "wasm32"))))]
mod lower {
    pub mod host {
        pub use crate::bindings::parla::plugin::host::*;
        use crate::types::Error;
        pub fn t(key: &str, args: &[(&str, &str)]) -> Result<String, Error> {
            let args: Vec<(String, String)> = args
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect();
            crate::bindings::parla::plugin::host::t(key, &args)
        }
    }

    /// Generates a cursor adapter: same arguments, `cursor: Option<&str>`.
    macro_rules! cursored {
        ($iface:ident: $( fn $name:ident($($pre:ident: $pty:ty),*; $($post:ident: $qty:ty),*) -> $ret:ty; )*) => {
            $(
                pub fn $name($($pre: $pty,)* cursor: Option<&str>, $($post: $qty),*) -> Result<$ret, Error> {
                    crate::bindings::parla::plugin::$iface::$name($($pre,)* &cursor.map(str::to_owned), $($post),*)
                }
            )*
        };
    }

    pub mod workspace {
        // The generated module also re-exports the types it uses.
        pub use crate::bindings::parla::plugin::workspace::*;
        cursored!(workspace:
            fn list_conversations(ws: &str, filter: &ConversationFilter; limit: u32) -> ConversationPage;
            fn history(ws: &str, conversation: &str; limit: u32) -> MessagePage;
            fn replies(ws: &str, conversation: &str, thread: &str; limit: u32) -> MessagePage;
            fn search_messages(ws: &str, query: &str; limit: u32) -> MessagePage;
            fn search_files(ws: &str, query: &str; limit: u32) -> FilePage;
            fn list_reminders(ws: &str; limit: u32) -> ReminderPage;
            fn list_scheduled(ws: &str; limit: u32) -> ScheduledPage;
            fn list_saved(ws: &str; limit: u32) -> MessageRefPage;
            fn list_pins(ws: &str, conversation: &str; limit: u32) -> MessageRefPage;
        );
    }

    pub mod storage {
        pub use crate::bindings::parla::plugin::storage::*;
        cursored!(storage:
            fn keys(scope: &Scope, prefix: &str; limit: u32) -> KeyPage;
        );
    }

    pub use crate::bindings::parla::plugin::{composer, exec, http, secrets, settings, ui};
}

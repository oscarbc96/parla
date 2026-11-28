//! The host interfaces that are not built yet: `storage`, `workspace`, `ui`,
//! `composer` and `exec`. Each function returns `error.unsupported` until its
//! plan task lands; linking them now lets any SDK plugin instantiate.

use super::parla::plugin::types::*;
use super::parla::plugin::{composer, exec, storage, ui, workspace};
use crate::linker::PluginState;

/// Every method returns [`Error::Unsupported`].
macro_rules! unsupported {
    (
        $(
            fn $name:ident ( $( $arg:ident : $ty:ty ),* $(,)? ) -> $ret:ty;
        )*
    ) => {
        $(
            async fn $name(&mut self $(, $arg: $ty)*) -> Result<$ret, Error> {
                let _ = ($($arg,)*);
                Err(Error::Unsupported)
            }
        )*
    };
}

// `types` and `view` have no functions; their traits are empty.
impl super::parla::plugin::types::Host for PluginState {}
impl super::parla::plugin::view::Host for PluginState {}

// Not built yet: the plugin key-value store.
impl storage::Host for PluginState {
    unsupported! {
        fn get(scope: storage::Scope, key: String) -> Option<Vec<u8>>;
        fn set(scope: storage::Scope, key: String, value: Vec<u8>) -> ();
        fn delete(scope: storage::Scope, key: String) -> ();
        fn keys(scope: storage::Scope, prefix: String, cursor: Cursor, limit: u32) -> KeyPage;
        fn clear(scope: storage::Scope) -> ();
    }
}

// Not built yet: workspace reads and writes.
impl workspace::Host for PluginState {
    unsupported! {
        fn list_workspaces() -> Vec<Workspace>;
        fn get_context() -> Context;
        fn me(ws: WorkspaceId) -> User;
        fn list_conversations(ws: WorkspaceId, filter: ConversationFilter, cursor: Cursor, limit: u32) -> ConversationPage;
        fn get_conversation(ws: WorkspaceId, conversation: ConversationId) -> Conversation;
        fn history(ws: WorkspaceId, conversation: ConversationId, cursor: Cursor, limit: u32) -> MessagePage;
        fn replies(ws: WorkspaceId, conversation: ConversationId, thread: MessageId, cursor: Cursor, limit: u32) -> MessagePage;
        fn get_message(ws: WorkspaceId, message: MessageRef) -> Message;
        fn permalink(ws: WorkspaceId, message: MessageRef) -> String;
        fn get_user(ws: WorkspaceId, user: UserId) -> User;
        fn find_users(ws: WorkspaceId, query: String, limit: u32) -> Vec<User>;
        fn get_presence(ws: WorkspaceId, user: UserId) -> Presence;
        fn search_messages(ws: WorkspaceId, query: String, cursor: Cursor, limit: u32) -> MessagePage;
        fn search_files(ws: WorkspaceId, query: String, cursor: Cursor, limit: u32) -> FilePage;
        fn file_info(ws: WorkspaceId, file: FileId) -> File;
        fn file_bytes(ws: WorkspaceId, file: FileId, offset: u64, len: u32) -> Vec<u8>;
        fn list_reminders(ws: WorkspaceId, cursor: Cursor, limit: u32) -> ReminderPage;
        fn list_scheduled(ws: WorkspaceId, cursor: Cursor, limit: u32) -> ScheduledPage;
        fn list_saved(ws: WorkspaceId, cursor: Cursor, limit: u32) -> MessageRefPage;
        fn list_pins(ws: WorkspaceId, conversation: ConversationId, cursor: Cursor, limit: u32) -> MessageRefPage;
        fn my_status(ws: WorkspaceId) -> Status;
        fn send(ws: WorkspaceId, conversation: ConversationId, content: Content) -> MessageRef;
        fn reply(ws: WorkspaceId, conversation: ConversationId, thread: MessageId, content: Content, also_to_channel: bool) -> MessageRef;
        fn edit(ws: WorkspaceId, message: MessageRef, content: Content) -> ();
        fn delete(ws: WorkspaceId, message: MessageRef) -> ();
        fn react(ws: WorkspaceId, message: MessageRef, emoji: String) -> ();
        fn unreact(ws: WorkspaceId, message: MessageRef, emoji: String) -> ();
        fn add_reminder(ws: WorkspaceId, text: String, at: Timestamp) -> ReminderId;
        fn complete_reminder(ws: WorkspaceId, reminder: ReminderId) -> ();
        fn delete_reminder(ws: WorkspaceId, reminder: ReminderId) -> ();
        fn schedule(ws: WorkspaceId, conversation: ConversationId, content: Content, at: Timestamp) -> ScheduledId;
        fn cancel_scheduled(ws: WorkspaceId, scheduled: ScheduledId) -> ();
        fn upload(ws: WorkspaceId, conversation: ConversationId, name: String, bytes: Vec<u8>, comment: Option<String>) -> File;
        fn set_status(ws: WorkspaceId, status: Status) -> ();
        fn set_presence(ws: WorkspaceId, presence: Presence) -> ();
        fn snooze(ws: WorkspaceId, minutes: u32) -> ();
        fn end_snooze(ws: WorkspaceId) -> ();
        fn set_saved(ws: WorkspaceId, message: MessageRef, saved: bool) -> ();
        fn set_pinned(ws: WorkspaceId, message: MessageRef, pinned: bool) -> ();
        fn mark_read(ws: WorkspaceId, conversation: ConversationId, upto: MessageId) -> ();
    }
}

// Not built yet: panels, windows, toasts and navigation.
impl ui::Host for PluginState {
    unsupported! {
        fn open_panel(panel_id: String, tree: ui::ViewTree) -> ViewId;
        fn open_window(spec: ui::WindowSpec, tree: ui::ViewTree) -> ViewId;
        fn update(view: ViewId, tree: ui::ViewTree) -> ();
        fn patch(view: ViewId, node: NodeId, tree: ui::ViewTree) -> ();
        fn close(view: ViewId) -> ();
        fn refresh_after(view: ViewId, seconds: u32) -> ();
        fn toast(text: String, kind: ui::ToastKind) -> ();
        fn notify(title: String, body: String, target: ui::NotifyTarget) -> ();
        fn copy(text: String) -> ();
        fn open_url(url: String) -> ();
        fn open_conversation(ws: WorkspaceId, conversation: ConversationId, message: Option<MessageId>) -> ();
    }
}

// Not built yet: the composer the plugin was invoked from.
impl composer::Host for PluginState {
    unsupported! {
        fn draft() -> String;
        fn replace_draft(text: String) -> ();
        fn insert(text: String) -> ();
    }
}

// Not built yet: local programs with fixed commands.
impl exec::Host for PluginState {
    unsupported! {
        fn run(program: String, args: Vec<String>) -> exec::ExecOutput;
    }
}

//! The "Delete message?" dialog, shared by the main window and pop-outs: the
//! host owns an `Option<DeleteConfirm>` and calls [`open`], [`close`] and
//! [`render_scrim`].

use components::{ConfirmDialog, ConfirmEvent, Timeline};
use domain::MessageId;
use gpui::{Context, Entity, Focusable as _, IntoElement, Subscription, Window, div, prelude::*};
use themes::ActiveTheme as _;

use crate::session::WorkspaceSession;

/// An open delete confirmation: the message asked about and the timeline that
/// deletes it.
pub struct DeleteConfirm {
    dialog: Entity<ConfirmDialog>,
    timeline: Entity<Timeline<WorkspaceSession>>,
    id: MessageId,
    _sub: Subscription,
}

impl DeleteConfirm {
    pub fn dialog(&self) -> &Entity<ConfirmDialog> {
        &self.dialog
    }
}

/// A window that can show the delete confirmation.
pub trait DeleteConfirmHost: Sized + 'static {
    fn delete_confirm_slot(&mut self) -> &mut Option<DeleteConfirm>;
}

/// Opens the dialog for message `id`, previewing `text` (plain, one line;
/// empty for a message without text), with focus on Delete.
pub fn open<V: DeleteConfirmHost>(
    host: &mut V,
    timeline: Entity<Timeline<WorkspaceSession>>,
    id: MessageId,
    text: String,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    if host.delete_confirm_slot().is_some() {
        return;
    }
    let l = i18n::gpui::localizer(cx);
    let title = t!(l, "shell-delete-message-title").into_owned();
    let ok = t!(l, "shell-delete-message-confirm").into_owned();
    let dialog = cx.new(|cx| {
        ConfirmDialog::new(title, String::new(), ok, cx)
            .danger(true)
            .preview(text)
            .delete_confirms()
    });
    let focus = dialog.read(cx).confirm_focus();
    let sub = cx.subscribe_in(&dialog, window, |this, _, ev: &ConfirmEvent, window, cx| {
        close(this, *ev == ConfirmEvent::Confirm, window, cx);
    });
    window.focus(&focus, cx);
    *host.delete_confirm_slot() = Some(DeleteConfirm {
        dialog,
        timeline,
        id,
        _sub: sub,
    });
    cx.notify();
}

/// Answers and closes the dialog, if open. Delete runs the timeline's delete
/// only while that message is still the one waiting for an answer; either
/// way focus returns to the timeline.
pub fn close<V: DeleteConfirmHost>(
    host: &mut V,
    confirmed: bool,
    window: &mut Window,
    cx: &mut Context<V>,
) {
    let Some(pane) = host.delete_confirm_slot().take() else {
        return;
    };
    pane.timeline.update(cx, |t, cx| {
        if confirmed && t.pending_delete() == Some(&pane.id) {
            t.confirm_delete(cx);
        } else {
            t.cancel_delete(cx);
        }
    });
    let focus = pane.timeline.read(cx).focus_handle(cx);
    window.focus(&focus, cx);
    cx.notify();
}

/// The dimmed overlay with the dialog on top; a click outside declines.
pub fn render_scrim<V: DeleteConfirmHost>(
    pane: &DeleteConfirm,
    cx: &mut Context<V>,
) -> impl IntoElement {
    let overlay = themes::parla_tokens(cx.theme()).bg_overlay;
    div()
        .id("delete-confirm-scrim")
        .debug_selector(|| "delete-confirm-dialog".into())
        .absolute()
        .inset_0()
        .flex()
        .justify_center()
        .items_start()
        .px(themes::metrics::SPACE_6)
        .pt(themes::metrics::LAYOUT_DIALOG_TOP)
        .bg(overlay)
        .occlude()
        .child(
            div()
                .w_full()
                .max_w(themes::metrics::LAYOUT_DIALOG_SM)
                .on_mouse_down_out(
                    cx.listener(|this, _, window, cx| close(this, false, window, cx)),
                )
                .child(pane.dialog.clone()),
        )
}

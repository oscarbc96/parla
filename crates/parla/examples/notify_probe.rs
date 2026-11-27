//! Manual probe: show a system notification, report responses.
//! Run from the bundle (copy the binary in) and from `cargo run --example notify_probe`.

use std::time::Duration;

use gpui::{App, SystemNotification, SystemNotificationAction};
use gpui_platform::application;

fn main() {
    application().run(|cx: &mut App| {
        cx.set_app_identity("com.justparla.parla", "Parla");
        cx.on_system_notification_response(|r, _cx| {
            println!("response tag={} action={:?}", r.tag, r.action_id);
        });
        cx.show_system_notification(SystemNotification {
            tag: "probe-1".into(),
            title: "Parla probe".into(),
            body: "Synthetic notification".into(),
            actions: vec![SystemNotificationAction {
                id: "reply".into(),
                label: "Reply".into(),
            }],
        });
        println!("posted probe-1");
        cx.spawn(async move |cx| {
            cx.background_executor()
                .timer(Duration::from_secs(10))
                .await;
            cx.update(|cx| {
                drop(cx.dismiss_system_notification("probe-1"));
                println!("dismissed probe-1");
            });
            cx.background_executor().timer(Duration::from_secs(3)).await;
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}

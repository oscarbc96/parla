//! Manual probe: a tray-icon status item created on the main thread
//! after GPUI starts. Tray UI itself is M6; this only records coexistence.

use std::time::Duration;

use gpui::App;
use gpui_platform::application;
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};

fn main() {
    application().run(|cx: &mut App| {
        let menu = Menu::new();
        let item = MenuItem::new("Probe item", true, None);
        menu.append(&item).expect("append menu item");
        let rgba = [38u8, 70, 83, 255].repeat(16 * 16);
        let icon = Icon::from_rgba(rgba, 16, 16).expect("icon");
        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Parla probe")
            .with_icon(icon)
            .build()
            .expect("tray");
        println!("tray created");
        cx.spawn(async move |cx| {
            let _tray = tray;
            for _ in 0..120 {
                cx.background_executor()
                    .timer(Duration::from_millis(500))
                    .await;
                while let Ok(ev) = MenuEvent::receiver().try_recv() {
                    println!("menu event {:?}", ev.id);
                }
            }
            cx.update(|cx| cx.quit());
        })
        .detach();
    });
}

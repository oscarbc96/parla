//! Startup ordering: slow platform probes must not run before the first
//! paint. The disk-encryption probe runs off the main thread and its notice
//! lands afterwards.

use gpui::TestAppContext;
use parla::store_banner::warn_if_unencrypted_later;
use parla::toast::ConfigBanner;

#[gpui::test]
fn encryption_probe_does_not_block_and_notice_arrives_later(cx: &mut TestAppContext) {
    cx.update(|cx| {
        i18n::gpui::init(
            cx,
            &[parla::catalog()],
            i18n::LocalePref::Code("en".parse().unwrap()),
        );
        cx.set_global(ConfigBanner::default());
    });
    let (tx, rx) = std::sync::mpsc::channel::<()>();
    cx.update(|cx| {
        warn_if_unencrypted_later(cx, move || {
            // Blocks until the test lets it go: the caller must not wait.
            let _ = rx.recv();
            Some(false)
        });
    });
    // Returned without the probe having finished: nothing shown yet.
    cx.update(|cx| assert!(cx.global::<ConfigBanner>().warnings.is_empty()));
    tx.send(()).unwrap();
    cx.executor().allow_parking();
    cx.run_until_parked();
    cx.update(|cx| {
        assert_eq!(
            cx.global::<ConfigBanner>().warnings,
            vec![
                "Full-disk encryption is off: image caches are stored unencrypted on this device."
                    .to_string()
            ]
        );
    });
}

#[gpui::test]
fn encrypted_disk_adds_no_notice(cx: &mut TestAppContext) {
    cx.update(|cx| {
        i18n::gpui::init(
            cx,
            &[parla::catalog()],
            i18n::LocalePref::Code("en".parse().unwrap()),
        );
        cx.set_global(ConfigBanner::default());
    });
    cx.update(|cx| warn_if_unencrypted_later(cx, || Some(true)));
    cx.executor().allow_parking();
    cx.run_until_parked();
    cx.update(|cx| assert!(cx.global::<ConfigBanner>().warnings.is_empty()));
}

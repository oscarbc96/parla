use std::io::Cursor;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use components::media::{
    Animation, ImageCache, ImageState, MediaError, MediaFetcher, decode, key_hash,
    prune_disk_cache, should_play,
};
use domain::ImageRef;
use futures::future::{BoxFuture, ready};
use gpui::{BorrowAppContext, TestAppContext, px, size};
use image::{Frame, ImageFormat, Rgba, RgbaImage};
use settings::GifAutoplay;

fn png(w: u32, h: u32) -> Vec<u8> {
    let img = RgbaImage::from_pixel(w, h, Rgba([200, 30, 30, 255]));
    let mut out = Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png).unwrap();
    out.into_inner()
}

fn gif(frames: u32) -> Vec<u8> {
    gif_delay(frames, 100)
}

fn gif_delay(frames: u32, ms: u64) -> Vec<u8> {
    let mut out = Vec::new();
    {
        let mut enc = image::codecs::gif::GifEncoder::new(&mut out);
        for i in 0..frames {
            let img = RgbaImage::from_pixel(8, 8, Rgba([(i * 60) as u8, 0, 0, 255]));
            let f = Frame::from_parts(
                img,
                0,
                0,
                image::Delay::from_saturating_duration(Duration::from_millis(ms)),
            );
            enc.encode_frame(f).unwrap();
        }
    }
    out
}

struct Counting {
    calls: AtomicUsize,
    bytes: Vec<u8>,
}

impl MediaFetcher for Counting {
    fn fetch(&self, _: &ImageRef, max: u64) -> BoxFuture<'static, Result<Vec<u8>, MediaError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let r = if self.bytes.len() as u64 > max {
            Err(MediaError::TooLarge)
        } else {
            Ok(self.bytes.clone())
        };
        Box::pin(ready(r))
    }
}

fn web(n: u32) -> ImageRef {
    ImageRef::Web(format!("https://example.invalid/{n}.png").into())
}

fn cache(
    cx: &mut TestAppContext,
    dir: &std::path::Path,
    bytes: Vec<u8>,
    budget: usize,
) -> Arc<Counting> {
    let f = Arc::new(Counting {
        calls: AtomicUsize::new(0),
        bytes,
    });
    let c = ImageCache::new(f.clone(), dir.to_path_buf(), budget);
    cx.update(|cx| cx.set_global(c));
    f
}

fn state(cx: &mut TestAppContext, src: &ImageRef, w: f32, h: f32, scale: f32) -> ImageState {
    cx.update(|cx| {
        cx.update_global::<ImageCache, _>(|c, cx| c.get(src, size(px(w), px(h)), scale, cx))
    })
}

#[gpui::test]
fn budget_evicts_lru(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    // 256x512 BGRA = 512 KB each; budget 1 MB.
    cache(cx, dir.path(), png(256, 512), 1024 * 1024);
    for n in 0..3 {
        let _ = state(cx, &web(n), 256.0, 512.0, 1.0);
        cx.run_until_parked();
    }
    assert!(matches!(
        state(cx, &web(2), 256.0, 512.0, 1.0),
        ImageState::Ready(_)
    ));
    assert!(matches!(
        state(cx, &web(1), 256.0, 512.0, 1.0),
        ImageState::Ready(_)
    ));
    // The first image was the least recently used and is gone.
    assert!(matches!(
        state(cx, &web(0), 256.0, 512.0, 1.0),
        ImageState::Loading
    ));
}

#[gpui::test]
fn one_url_at_three_sizes_downloads_once(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let f = cache(cx, dir.path(), png(200, 200), 64 * 1024 * 1024);
    for side in [24.0, 36.0, 72.0] {
        assert!(matches!(
            state(cx, &web(0), side, side, 2.0),
            ImageState::Loading
        ));
    }
    cx.run_until_parked();
    for side in [24.0, 36.0, 72.0] {
        let ImageState::Ready(img) = state(cx, &web(0), side, side, 2.0) else {
            panic!("{side} not ready");
        };
        assert_eq!(img.size(0).width.0, (side * 2.0) as i32);
    }
    assert_eq!(f.calls.load(Ordering::SeqCst), 1, "one download per URL");
}

#[gpui::test]
fn decodes_at_display_size(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    cache(cx, dir.path(), png(2000, 2000), 64 * 1024 * 1024);
    assert!(matches!(
        state(cx, &web(0), 100.0, 100.0, 2.0),
        ImageState::Loading
    ));
    cx.run_until_parked();
    let ImageState::Ready(img) = state(cx, &web(0), 100.0, 100.0, 2.0) else {
        panic!("not ready");
    };
    let s = img.size(0);
    assert_eq!((s.width.0, s.height.0), (200, 200));
}

#[gpui::test]
fn disk_cache_hit_skips_fetch(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let f = cache(cx, dir.path(), png(16, 16), 64 * 1024 * 1024);
    let _ = state(cx, &web(0), 16.0, 16.0, 1.0);
    cx.run_until_parked();
    assert_eq!(f.calls.load(Ordering::SeqCst), 1);
    // A fresh cache (empty memory) over the same directory reads the file.
    let f2 = cache(cx, dir.path(), Vec::new(), 64 * 1024 * 1024);
    let _ = state(cx, &web(0), 16.0, 16.0, 1.0);
    cx.run_until_parked();
    assert_eq!(f2.calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        state(cx, &web(0), 16.0, 16.0, 1.0),
        ImageState::Ready(_)
    ));
}

#[gpui::test]
fn decode_failure_is_failed_not_panic(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    cache(cx, dir.path(), b"not an image".to_vec(), 1024 * 1024);
    let _ = state(cx, &web(0), 16.0, 16.0, 1.0);
    cx.run_until_parked();
    assert!(matches!(
        state(cx, &web(0), 16.0, 16.0, 1.0),
        ImageState::Failed
    ));
}

#[test]
fn gif_animates_only_when_visible_and_focused() {
    // Gate: not painted, window inactive, or setting never -> frozen.
    assert!(!should_play(GifAutoplay::Visible, false, true, false));
    assert!(!should_play(GifAutoplay::Visible, true, false, false));
    assert!(should_play(GifAutoplay::Visible, true, true, false));
    assert!(!should_play(GifAutoplay::Hover, true, true, false));
    assert!(should_play(GifAutoplay::Hover, true, true, true));
    assert!(!should_play(GifAutoplay::Never, true, true, true));

    let mut a = Animation::new(gif(3), 8, 8).unwrap();
    let t0 = Instant::now();
    assert_eq!(a.frame_index(), 0);
    // Gate closed: time passes, the frame does not.
    assert_eq!(a.tick(t0, false), None);
    assert_eq!(a.tick(t0 + Duration::from_secs(5), false), None);
    assert_eq!(a.frame_index(), 0);
    // Gate open: the first tick starts the clock, later ticks advance.
    assert!(a.tick(t0 + Duration::from_secs(6), true).is_some());
    assert_eq!(a.frame_index(), 0);
    assert!(a.tick(t0 + Duration::from_millis(6200), true).is_some());
    assert_eq!(a.frame_index(), 1);
    // Closing the gate freezes it again.
    assert_eq!(a.tick(t0 + Duration::from_secs(9), false), None);
    assert_eq!(a.frame_index(), 1);
}

#[test]
fn long_gif_keeps_a_bounded_number_of_frames() {
    // 8x8 BGRA frames are 256 bytes; a 1 KB cap keeps four of 40.
    let mut a = Animation::with_cap(gif(40), 8, 8, 1024).unwrap();
    let t0 = Instant::now();
    let mut now = t0;
    a.tick(now, true);
    let mut max_index = 0;
    for _ in 0..100 {
        now += Duration::from_millis(150);
        a.tick(now, true);
        max_index = max_index.max(a.frame_index());
        assert!(a.memory_bytes() <= 1024, "{}", a.memory_bytes());
    }
    assert_eq!(max_index, 39, "plays every frame");
    // 100 steps over a 40-frame loop wrap back around.
    assert_eq!(a.frame_index(), 100 % 40);
}

#[test]
fn short_gif_replays_from_memory() {
    let mut a = Animation::new(gif(3), 8, 8).unwrap();
    let mut now = Instant::now();
    a.tick(now, true);
    for i in 1..=7 {
        now += Duration::from_millis(150);
        a.tick(now, true);
        assert_eq!(a.frame_index(), i % 3);
    }
    assert_eq!(a.memory_bytes(), 3 * 256);
}

#[gpui::test]
fn gif_frames_count_against_the_image_budget(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    cache(cx, dir.path(), gif(30), 64 * 1024 * 1024);
    let src = web(9);
    let _ = state(cx, &src, 8.0, 8.0, 1.0);
    cx.run_until_parked();
    let before = cx.update(|cx| cx.global::<ImageCache>().used_bytes());
    let anim = cx
        .update(|cx| {
            cx.update_global::<ImageCache, _>(|c, _| c.animation(&src, size(px(8.0), px(8.0)), 1.0))
        })
        .unwrap();
    let mut now = Instant::now();
    for _ in 0..30 {
        anim.borrow_mut().tick(now, true);
        now += Duration::from_millis(150);
    }
    cx.update(|cx| {
        cx.update_global::<ImageCache, _>(|c, _| c.animation(&src, size(px(8.0), px(8.0)), 1.0))
    });
    let after = cx.update(|cx| cx.global::<ImageCache>().used_bytes());
    assert!(after >= before + 30 * 256 - 256, "{before} -> {after}");
}

#[test]
fn disk_prune_removes_oldest_to_fit_cap() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("p/w");
    std::fs::create_dir_all(&sub).unwrap();
    for (i, name) in ["a", "b", "c"].iter().enumerate() {
        let p = sub.join(name);
        std::fs::write(&p, vec![0u8; 100]).unwrap();
        let t = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1000 + i as u64);
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }
    prune_disk_cache(dir.path(), 250);
    assert!(!sub.join("a").exists());
    assert!(sub.join("b").exists() && sub.join("c").exists());
}

#[test]
fn key_hash_is_stable_128_bit_hex() {
    let h = key_hash("https://example.invalid/a.png");
    assert_eq!(h.len(), 32);
    assert_eq!(h, key_hash("https://example.invalid/a.png"));
    assert_ne!(h, key_hash("https://example.invalid/b.png"));
    // FNV-1a 128 of the empty string is the offset basis.
    assert_eq!(key_hash(""), "6c62272e07bb014262b821756295c58d");
}

#[test]
fn decode_rejects_garbage() {
    assert_eq!(decode(b"nope", 10, 10).err(), Some(MediaError::Decode));
}

#[gpui::test]
fn frozen_motion_keeps_gif_on_frame_zero(cx: &mut TestAppContext) {
    use components::Motion;
    use components::media::animation_for;

    let dir = tempfile::tempdir().unwrap();
    cache(cx, dir.path(), gif(3), 64 * 1024 * 1024);
    let src = web(7);
    let _ = state(cx, &src, 8.0, 8.0, 1.0);
    cx.run_until_parked();
    assert!(matches!(
        state(cx, &src, 8.0, 8.0, 1.0),
        ImageState::Ready(_)
    ));
    let boxed = size(px(8.0), px(8.0));
    // Live: an animation exists.
    cx.update(|cx| {
        assert!(animation_for(cx, &src, boxed, 1.0, GifAutoplay::Visible).is_some());
    });
    // Frozen: none is handed out, whatever the clock does; nothing ticks.
    cx.update(|cx| components::set_motion(cx, Motion::Frozen));
    cx.executor().advance_clock(Duration::from_secs(2));
    cx.run_until_parked();
    cx.update(|cx| {
        assert!(animation_for(cx, &src, boxed, 1.0, GifAutoplay::Visible).is_none());
    });
}

/// Root view that counts its renders and shows `n` animated GIFs.
struct Gifs {
    renders: Arc<AtomicUsize>,
    srcs: Vec<ImageRef>,
}

impl gpui::Render for Gifs {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement, Styled, div};
        self.renders.fetch_add(1, Ordering::SeqCst);
        div().flex().children(self.srcs.iter().map(|s| {
            components::media::MediaImage::new(s.clone(), "e")
                .max_size(20.0, 20.0)
                .fixed()
        }))
    }
}

fn redraws_per_second(cx: &mut TestAppContext, frame_ms: u64) -> usize {
    redraws(cx, frame_ms, 8, true, None)
}

/// Renders in one simulated second of `n` animated emoji; `active` is the
/// window's focus, `autoplay` an optional `gifs.autoplay_in_messages`.
fn redraws(
    cx: &mut TestAppContext,
    frame_ms: u64,
    n: u32,
    active: bool,
    autoplay: Option<&str>,
) -> usize {
    let dir = tempfile::tempdir().unwrap();
    cache(cx, dir.path(), gif_delay(3, frame_ms), 64 * 1024 * 1024);
    if let Some(mode) = autoplay {
        let paths = settings::ConfigPaths::at(dir.path().join("cfg"));
        std::fs::create_dir_all(&paths.root).unwrap();
        std::fs::write(
            &paths.settings,
            format!(r#"{{"gifs": {{"autoplay_in_messages": "{mode}"}}}}"#),
        )
        .unwrap();
        cx.update(|cx| cx.set_global(settings::SettingsStore::load(paths)));
    }
    let srcs: Vec<ImageRef> = (0..n).map(web).collect();
    let renders = Arc::new(AtomicUsize::new(0));
    let (r, s) = (renders.clone(), srcs);
    let (_view, vcx) = cx.add_window_view(move |window, _| {
        if active {
            window.activate_window();
        }
        Gifs {
            renders: r,
            srcs: s,
        }
    });
    vcx.run_until_parked();
    // Let the images load and the animations start.
    for _ in 0..5 {
        vcx.executor().advance_clock(Duration::from_millis(16));
        vcx.run_until_parked();
    }
    let before = renders.load(Ordering::SeqCst);
    // One simulated second at 60 Hz.
    for _ in 0..60 {
        vcx.executor().advance_clock(Duration::from_millis(16));
        vcx.run_until_parked();
        // A runaway loop never settles; stop measuring once it is obvious.
        if renders.load(Ordering::SeqCst) - before > 400 {
            break;
        }
    }
    renders.load(Ordering::SeqCst) - before
}

/// 30 animated emoji on screen (a busy channel): the whole window redraws at
/// most 10 times a second (15 here: the test clock steps by 16 ms and
/// image loads add a few), however fast the GIFs are.
#[gpui::test]
fn thirty_animated_emoji_stay_within_the_redraw_budget(cx: &mut TestAppContext) {
    let drawn = redraws(cx, 20, 30, true, None);
    println!("emoji30.redraws measured: {drawn} per second");
    assert!(drawn <= 15, "redraws per second with 30 emoji: {drawn}");
    assert!(drawn >= 1, "the animation still advances");
}

/// A window that is not active (background, occluded) does not redraw for
/// animation at all.
#[gpui::test]
fn animated_emoji_do_not_redraw_in_an_inactive_window(cx: &mut TestAppContext) {
    let drawn = redraws(cx, 20, 30, false, None);
    println!("emoji30.inactive redraws measured: {drawn} per second");
    assert_eq!(drawn, 0, "inactive window redraws per second: {drawn}");
}

/// `gifs.autoplay_in_messages = never` freezes emoji: no animation redraws.
#[gpui::test]
fn autoplay_never_setting_stops_emoji_redraws(cx: &mut TestAppContext) {
    let drawn = redraws(cx, 20, 30, true, Some("never"));
    assert_eq!(drawn, 0, "redraws per second with autoplay never: {drawn}");
}

/// Animated emoji must not turn into a render loop: with several GIFs on
/// screen the window redraws about as often as the frames change, not once
/// per display refresh and not once per GIF.
#[gpui::test]
fn animated_gifs_do_not_redraw_continuously(cx: &mut TestAppContext) {
    let drawn = redraws_per_second(cx, 100);
    println!("gif.redraws measured: {drawn} per second");
    assert!(drawn <= 20, "gif redraws per second: {drawn}");
    assert!(drawn >= 1, "the animation still advances");
}

/// Fast GIFs (20 ms frames) redraw on the shared 50 ms animation grid, not
/// every 20 ms (the bound leaves room for the test clock stepping by 16 ms).
#[gpui::test]
fn fast_gifs_redraw_on_a_shared_grid(cx: &mut TestAppContext) {
    let drawn = redraws_per_second(cx, 20);
    println!("gif.fast redraws measured: {drawn} per second");
    assert!(drawn <= 30, "fast gif redraws per second: {drawn}");
    assert!(drawn >= 1, "the animation still advances");
}

#[gpui::test]
fn corrupt_disk_file_is_deleted(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let src = web(0);
    let path = dir.path().join(key_hash("https://example.invalid/0.png"));
    std::fs::write(&path, b"torn").unwrap();
    cache(cx, dir.path(), png(16, 16), 1024 * 1024);
    let _ = state(cx, &src, 16.0, 16.0, 1.0);
    cx.run_until_parked();
    assert!(matches!(
        state(cx, &src, 16.0, 16.0, 1.0),
        ImageState::Failed
    ));
    assert!(!path.exists(), "the bad cache file is removed");
}

#[gpui::test]
fn failed_entries_are_evicted_by_the_budget(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    cache(cx, dir.path(), b"not an image".to_vec(), 2048);
    for n in 0..3 {
        let _ = state(cx, &web(n), 16.0, 16.0, 1.0);
        cx.run_until_parked();
    }
    // The oldest failure was evicted, so it loads again.
    assert!(matches!(
        state(cx, &web(0), 16.0, 16.0, 1.0),
        ImageState::Loading
    ));
}

struct Failing(MediaError);

impl MediaFetcher for Failing {
    fn fetch(&self, _: &ImageRef, _: u64) -> BoxFuture<'static, Result<Vec<u8>, MediaError>> {
        Box::pin(ready(Err(self.0.clone())))
    }
}

fn kinds_seen(cx: &mut TestAppContext, f: Arc<dyn MediaFetcher>) -> Vec<&'static str> {
    let dir = tempfile::tempdir().unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = seen.clone();
    let c = ImageCache::new(f, dir.path().to_path_buf(), 1 << 20)
        .with_failure_hook(Arc::new(move |e| sink.lock().unwrap().push(e.kind())));
    cx.update(|cx| cx.set_global(c));
    let _ = state(cx, &web(1), 36.0, 36.0, 2.0);
    cx.run_until_parked();
    // A second draw inside the TTL neither refetches nor reports again.
    let _ = state(cx, &web(1), 36.0, 36.0, 2.0);
    cx.run_until_parked();
    seen.lock().unwrap().clone()
}

#[gpui::test]
fn a_failed_fetch_is_reported_once_by_kind(cx: &mut TestAppContext) {
    let net = Arc::new(Failing(MediaError::Network("status 403".into())));
    assert_eq!(kinds_seen(cx, net), vec!["network"]);
    let refused = Arc::new(Failing(MediaError::Refused));
    assert_eq!(kinds_seen(cx, refused), vec!["refused"]);
}

#[gpui::test]
fn a_picture_that_does_not_decode_is_reported(cx: &mut TestAppContext) {
    let junk = Arc::new(Counting {
        calls: AtomicUsize::new(0),
        bytes: b"<svg/>".to_vec(),
    });
    assert_eq!(kinds_seen(cx, junk), vec!["decode"]);
}

#[gpui::test]
fn a_good_picture_reports_nothing(cx: &mut TestAppContext) {
    let ok = Arc::new(Counting {
        calls: AtomicUsize::new(0),
        bytes: png(8, 8),
    });
    assert!(kinds_seen(cx, ok).is_empty());
}

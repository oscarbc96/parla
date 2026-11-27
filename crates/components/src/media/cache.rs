//! Memory LRU (by decoded bytes) over a plain-file disk cache.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

use domain::ImageRef;
use futures::FutureExt as _;
use futures::future::BoxFuture;
use gpui::{App, AsyncApp, BorrowAppContext, Global, Pixels, RenderImage, Size};
use lru::LruCache;

use super::anim::Animation;
use super::decode::decode;
use super::{MAX_IMAGE_BYTES, MediaError, MediaFetcher};

/// Disk cache cap, enforced by [`prune_disk_cache`] at startup only.
pub const DISK_CAP_BYTES: u64 = 256 * 1024 * 1024;

/// Hex of the 128-bit FNV-1a of `s`.
pub fn key_hash(s: &str) -> String {
    format!("{:032x}", hash128(s))
}

/// The 128-bit FNV-1a of `s`: a collision-free-in-practice key that needs no allocation.
fn hash128(s: &str) -> u128 {
    const OFFSET: u128 = 0x6c62272e07bb014262b821756295c58d;
    const PRIME: u128 = 0x0000000001000000000000000000013b;
    let mut h = OFFSET;
    for b in s.bytes() {
        h ^= u128::from(b);
        h = h.wrapping_mul(PRIME);
    }
    h
}

pub(crate) fn src_key(src: &ImageRef) -> &str {
    match src {
        ImageRef::Provider(u) => &u.0,
        ImageRef::Web(u) => u,
    }
}

/// Removes the oldest files under `root` until the total size is at most
/// `cap`; returns the total left.
pub fn prune_disk_cache(root: &Path, cap: u64) -> u64 {
    let mut files: Vec<(SystemTime, u64, PathBuf)> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let Ok(md) = e.metadata() else { continue };
            if md.is_dir() {
                stack.push(e.path());
            } else if md.is_file() {
                files.push((
                    md.modified().unwrap_or(SystemTime::UNIX_EPOCH),
                    md.len(),
                    e.path(),
                ));
            }
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    files.sort();
    for (_, len, p) in files {
        if total <= cap {
            break;
        }
        if std::fs::remove_file(&p).is_ok() {
            total -= len;
        }
    }
    total
}

/// Keeps a disk cache folder within its cap as files are added: the first
/// write sizes the folder once, each write adds its bytes, and passing the cap
/// removes the oldest files (by modification time). Called off the main thread.
pub struct DiskBudget {
    root: PathBuf,
    cap: u64,
    used: std::sync::Mutex<Option<u64>>,
}

impl DiskBudget {
    pub fn new(root: PathBuf, cap: u64) -> Self {
        Self {
            root,
            cap,
            used: std::sync::Mutex::new(None),
        }
    }

    /// Counts `added` bytes just written under the root; prunes past the cap.
    pub fn record(&self, added: u64) {
        let Ok(mut used) = self.used.lock() else {
            return;
        };
        let total = match *used {
            Some(u) => u + added,
            // The first write: size what is there (it includes this file).
            None => prune_disk_cache(&self.root, u64::MAX),
        };
        *used = Some(if total > self.cap {
            prune_disk_cache(&self.root, self.cap)
        } else {
            total
        });
    }
}

/// Nominal memory cost of a failed entry, so the budget can evict them.
const FAILED_COST: usize = 1024;

/// How long a failed fetch hides the image before the next draw retries it.
const FAILED_TTL: Duration = Duration::from_secs(30);

/// A failed entry from `at` is stale at `now`.
fn failed_expired(at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(at) >= FAILED_TTL
}

/// Writes `bytes` to a sibling temp file and renames it over `path`, so a
/// reader never sees a torn file.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = PathBuf::from(tmp);
    let done = std::fs::write(&tmp, bytes).and_then(|()| std::fs::rename(&tmp, path));
    if done.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    done
}

type Decoded3 = (Arc<RenderImage>, usize, Option<Arc<Vec<u8>>>);

#[derive(Clone)]
pub enum ImageState {
    Loading,
    Ready(Arc<RenderImage>),
    Failed,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Key {
    /// [`hash128`] of the URL; looking a frame up allocates nothing.
    src: u128,
    w: u32,
    h: u32,
}

type Download = futures::future::Shared<BoxFuture<'static, Result<Arc<Vec<u8>>, MediaError>>>;

struct Entry {
    state: ImageState,
    bytes: usize,
    /// `bytes` without the animation's decoded frames.
    base: usize,
    /// Original GIF bytes, kept so an [`Animation`] can decode further frames.
    gif: Option<Arc<Vec<u8>>>,
    anim: Option<Rc<RefCell<Animation>>>,
    /// When a fetch failed; such an entry expires after `FAILED_TTL`.
    failed_at: Option<Instant>,
}

pub struct ImageCache {
    fetcher: Arc<dyn MediaFetcher>,
    /// `<cache_dir>/media/<provider>/<workspace>`.
    disk: PathBuf,
    /// Keeps `disk` within [`DISK_CAP_BYTES`] as files are written.
    disk_budget: Arc<DiskBudget>,
    budget: usize,
    used: usize,
    lru: LruCache<Key, Entry>,
    inflight: HashSet<Key>,
    /// Downloads under way, one per URL: every size of an image waits on the
    /// same fetch and then decodes at its own size.
    downloads: HashMap<u128, Download>,
    /// When the one pending animation wake-up fires, if one is armed.
    wake: Option<Instant>,
    /// Origin of the shared animation wake-up grid (app clock).
    epoch: Option<Instant>,
    /// Told once per failed fetch or decode (never the URL), so the app can log it.
    on_failure: Option<FailureHook>,
    /// Told once per image that loaded (a fetch, not a disk hit).
    on_success: Option<SuccessHook>,
}

/// Called when an image was fetched and decoded.
pub type SuccessHook = Arc<dyn Fn() + Send + Sync>;

/// Called with the error of an image that failed to load.
pub type FailureHook = Arc<dyn Fn(&MediaError) + Send + Sync>;

impl Global for ImageCache {}

fn device_px(v: Pixels, scale: f32) -> u32 {
    ((f32::from(v) * scale).ceil().max(1.0)) as u32
}

impl ImageCache {
    pub fn new(fetcher: Arc<dyn MediaFetcher>, disk: PathBuf, budget_bytes: usize) -> Self {
        Self {
            fetcher,
            disk_budget: Arc::new(DiskBudget::new(disk.clone(), DISK_CAP_BYTES)),
            disk,
            budget: budget_bytes,
            used: 0,
            lru: LruCache::unbounded(),
            inflight: HashSet::new(),
            downloads: HashMap::new(),
            wake: None,
            epoch: None,
            on_failure: None,
            on_success: None,
        }
    }

    /// Reports every loaded image to `hook`.
    pub fn with_success_hook(mut self, hook: SuccessHook) -> Self {
        self.on_success = Some(hook);
        self
    }

    /// Reports every failed load to `hook`.
    pub fn with_failure_hook(mut self, hook: FailureHook) -> Self {
        self.on_failure = Some(hook);
        self
    }

    fn key(src: &ImageRef, size: Size<Pixels>, scale: f32) -> Key {
        Key {
            src: hash128(src_key(src)),
            w: device_px(size.width, scale),
            h: device_px(size.height, scale),
        }
    }

    /// The decoded image for `src` at `size` (logical pixels) on a display with
    /// `scale`. A miss starts the fetch and decode in the background and
    /// refreshes windows when done; until then the state is `Loading`.
    pub fn get(
        &mut self,
        src: &ImageRef,
        size: Size<Pixels>,
        scale: f32,
        cx: &mut App,
    ) -> ImageState {
        let key = Self::key(src, size, scale);
        if let Some(e) = self.lru.get(&key) {
            let stale = e
                .failed_at
                .is_some_and(|at| failed_expired(at, Instant::now()));
            if !stale {
                return e.state.clone();
            }
            if let Some(old) = self.lru.pop(&key) {
                self.used -= old.bytes;
            }
        }
        if self.inflight.insert(key) {
            let disk_budget = self.disk_budget.clone();
            let path = self.disk.join(format!("{:032x}", key.src));
            let src = src.clone();
            let k = key;
            cx.spawn(async move |cx: &mut AsyncApp| {
                let bg = cx.background_executor().clone();
                let p = path.clone();
                let cached = bg.spawn(async move { std::fs::read(p).ok() }).await;
                let (raw, from_disk) = match cached {
                    Some(b) => (Ok(b), true),
                    None => {
                        let url = k.src;
                        let dl = cx.update(|cx| {
                            cx.update_global::<ImageCache, _>(|c, _| c.download(url, &src))
                        });
                        let got = dl.await;
                        let got = got.map(|a| Arc::try_unwrap(a).unwrap_or_else(|a| (*a).clone()));
                        (got, false)
                    }
                };
                let (w, h) = (k.w, k.h);
                let result = match raw {
                    Err(e) => Err(e),
                    Ok(bytes) => {
                        bg.spawn(async move {
                            if !from_disk {
                                if let Some(dir) = path.parent() {
                                    let _ = std::fs::create_dir_all(dir);
                                }
                                // Another size of the same image may have written it.
                                if !path.is_file() && write_atomic(&path, &bytes).is_ok() {
                                    disk_budget.record(bytes.len() as u64);
                                }
                            }
                            let decoded = decode(&bytes, w, h);
                            if decoded.is_err() && from_disk {
                                // A bad cached file: drop it so the next start refetches.
                                let _ = std::fs::remove_file(&path);
                            }
                            decoded.map(|d| {
                                let gif = d.gif.then(|| Arc::new(bytes));
                                (d.image, d.bytes, gif)
                            })
                        })
                        .await
                    }
                };
                cx.update(|cx| {
                    if cx.has_global::<ImageCache>() {
                        cx.update_global::<ImageCache, _>(|c, _| c.finish(k, result));
                    }
                    cx.refresh_windows();
                });
            })
            .detach();
        }
        ImageState::Loading
    }

    /// The download of `src`, started once and shared by every size waiting on it.
    fn download(&mut self, url: u128, src: &ImageRef) -> Download {
        let fetcher = self.fetcher.clone();
        self.downloads
            .entry(url)
            .or_insert_with(|| {
                let f = fetcher.fetch(src, MAX_IMAGE_BYTES);
                async move { f.await.map(Arc::new) }.boxed().shared()
            })
            .clone()
    }

    fn finish(&mut self, key: Key, result: Result<Decoded3, MediaError>) {
        self.inflight.remove(&key);
        // The last size waiting on a download lets it go.
        if !self.inflight.iter().any(|k| k.src == key.src) {
            self.downloads.remove(&key.src);
        }
        let entry = match result {
            Ok((image, bytes, gif)) => {
                if let Some(h) = &self.on_success {
                    h();
                }
                Entry {
                    bytes: bytes + gif.as_ref().map_or(0, |g| g.len()),
                    base: bytes + gif.as_ref().map_or(0, |g| g.len()),
                    state: ImageState::Ready(image),
                    gif,
                    anim: None,
                    failed_at: None,
                }
            }
            Err(e) => {
                if let Some(h) = &self.on_failure {
                    h(&e);
                }
                Entry {
                    state: ImageState::Failed,
                    bytes: FAILED_COST,
                    base: FAILED_COST,
                    gif: None,
                    anim: None,
                    failed_at: Some(Instant::now()),
                }
            }
        };
        self.used += entry.bytes;
        if let Some((_, old)) = self.lru.push(key, entry) {
            self.used -= old.bytes;
        }
        self.evict();
    }

    fn evict(&mut self) {
        while self.used > self.budget && self.lru.len() > 1 {
            if let Some((_, e)) = self.lru.pop_lru() {
                self.used -= e.bytes;
            }
        }
    }

    /// Whether `src`'s bytes are already in the disk cache (no fetch needed).
    pub fn on_disk(&self, src: &ImageRef) -> bool {
        self.disk.join(key_hash(src_key(src))).is_file()
    }

    /// The fetcher the cache loads through (the media viewer shares it).
    pub fn fetcher(&self) -> Arc<dyn MediaFetcher> {
        self.fetcher.clone()
    }

    /// The disk cache folder (the media viewer shares it).
    pub fn disk_dir(&self) -> &Path {
        &self.disk
    }

    /// The disk cache's budget (the media viewer's writes count against it).
    pub fn disk_budget(&self) -> Arc<DiskBudget> {
        self.disk_budget.clone()
    }

    /// Decoded bytes counted against the budget (images and GIF frames).
    pub fn used_bytes(&self) -> usize {
        self.used
    }

    /// Origin of the shared wake-up grid: the first app-clock time asked for.
    pub fn grid_epoch(&mut self, now: Instant) -> Instant {
        *self.epoch.get_or_insert(now)
    }

    /// Asks for a redraw at `due`. Returns true when the caller must arm a
    /// timer for it; false when an earlier wake-up is already armed (the
    /// redraw it causes paints every animation again, which asks anew).
    /// Without this, every painted GIF armed its own timer on every paint and
    /// the timers multiplied into a redraw on every display refresh.
    pub fn arm_wake(&mut self, due: Instant) -> bool {
        match self.wake {
            Some(armed) if armed <= due => false,
            _ => {
                self.wake = Some(due);
                true
            }
        }
    }

    /// The timer armed for `due` fired. True when it is still the current
    /// wake-up and the windows should redraw.
    pub fn take_wake(&mut self, due: Instant) -> bool {
        if self.wake == Some(due) {
            self.wake = None;
            true
        } else {
            false
        }
    }

    /// The animation of a ready GIF, created on first use.
    pub fn animation(
        &mut self,
        src: &ImageRef,
        size: Size<Pixels>,
        scale: f32,
    ) -> Option<Rc<RefCell<Animation>>> {
        let key = Self::key(src, size, scale);
        let e = self.lru.get_mut(&key)?;
        if e.anim.is_none() {
            let bytes = e.gif.as_ref()?;
            e.anim = Animation::new(bytes.clone(), key.w, key.h)
                .ok()
                .map(|a| Rc::new(RefCell::new(a)));
        }
        let anim = e.anim.clone();
        // The animation's decoded frames count against the image budget; they
        // grow as it plays, so the entry is re-measured on every use.
        if let Some(a) = &anim {
            let want = e.base + a.borrow().memory_bytes();
            let old = std::mem::replace(&mut e.bytes, want);
            self.used = self.used + want - old;
            self.evict();
        }
        anim
    }
}

#[cfg(test)]
mod failed_ttl_tests {
    use super::*;

    #[test]
    fn failed_entries_expire_after_the_ttl() {
        let t0 = Instant::now();
        assert!(!failed_expired(t0, t0));
        assert!(!failed_expired(
            t0,
            t0 + FAILED_TTL - Duration::from_secs(1)
        ));
        assert!(failed_expired(t0, t0 + FAILED_TTL));
    }
}

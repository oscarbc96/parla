//! Scripted scenarios for `mise run perf:measure`: the real
//! window on the synthetic `perf` workspace, driven through GPUI input, with
//! the process sampling itself. Each run writes `<out>/<scenario>.json`.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use domain::test_support::FakeProvider;
use domain::test_support::fixture::{conversation_id, member_id};
use domain::{ConversationId, Message, MessageId, MessageTs, ProviderEvent};
use futures::channel::oneshot;
use gpui::{
    App, AppContext as _, AsyncApp, Keystroke, Modifiers, PlatformInput, ScrollDelta, ScrollEvent,
    ScrollSource, TouchPhase, WindowHandle, point, px,
};

use crate::perf::report::{Report, Sample, dropped_pct, late_pct, percentile, sample};
use crate::perf::{frames_drawn, input_frames, process_start};
use crate::root_view::{Phase, RootView};

/// Longest a scenario waits for one frame before failing instead of hanging.
const FRAME_STALL: Duration = Duration::from_secs(10);

/// Scenarios `perf:measure` accepts.
pub const SCENARIOS: &[&str] = &[
    "idle",
    "idle-hidden",
    "emoji",
    "emoji-hidden",
    "scroll",
    "scroll-input",
    "type",
    "switch",
    "start",
    "start-cold",
    "soak",
];

#[derive(Clone, Debug)]
pub struct Options {
    pub scenario: String,
    /// Duration of the timed part (`idle`, `idle-hidden`, `scroll`, `type`).
    pub seconds: u64,
    /// Duration of the `soak` scenario.
    pub hours: f64,
    pub out: PathBuf,
    /// Workspaces the fixture registers: 1 (`perf`) or 3 (`perf3`).
    pub workspaces: usize,
    /// Set on the children a `start` run spawns.
    pub child: bool,
}

impl Options {
    /// `--perf-scenario <name> [--seconds <n>] [--hours <n>] [--workspaces <1|3>] [--out <dir>]`.
    pub fn from_args(args: &[String]) -> Option<Options> {
        let value = |flag: &str| {
            args.iter()
                .position(|a| a == flag)
                .and_then(|i| args.get(i + 1))
                .cloned()
        };
        let scenario = value("--perf-scenario")?;
        Some(Options {
            seconds: value("--seconds").and_then(|s| s.parse().ok()).unwrap_or(
                if scenario.starts_with("idle") || scenario.starts_with("emoji") {
                    300
                } else {
                    10
                },
            ),
            hours: value("--hours").and_then(|s| s.parse().ok()).unwrap_or(8.0),
            out: value("--out").map_or_else(|| PathBuf::from("target/perf"), PathBuf::from),
            workspaces: value("--workspaces")
                .and_then(|s| s.parse().ok())
                .unwrap_or(1),
            child: args.iter().any(|a| a == "--perf-child"),
            scenario,
        })
    }
}

/// Points the app at the fixture and a scratch data and cache directory, so a
/// run never touches real data. Call before `parla::init`.
pub fn prepare_env(workspaces: usize) {
    if workspaces != 1 && workspaces != 3 {
        eprintln!("--workspaces takes 1 or 3");
        std::process::exit(2);
    }
    // SAFETY: called first thing in `main`, before any other thread exists.
    unsafe {
        std::env::set_var(
            "PARLA_FIXTURE",
            if workspaces == 3 { "perf3" } else { "perf" },
        );
        if std::env::var_os("PARLA_DATA_DIR").is_none() {
            let dir = std::env::temp_dir().join(format!("parla-perf-{}", std::process::id()));
            std::env::set_var("PARLA_DATA_DIR", dir.join("data"));
            std::env::set_var("PARLA_CACHE_DIR", dir.join("cache"));
        }
    }
}

/// `start` and `start-cold` need a warm store from an earlier launch: the
/// parent runs the app once to fill it, then once more to measure. Returns the
/// exit code when `opts` is such a parent run.
pub fn parent_if_needed(opts: &Options) -> Option<i32> {
    if opts.child || !opts.scenario.starts_with("start") {
        return None;
    }
    let dir = std::env::temp_dir().join(format!("parla-perf-start-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let exe = std::env::current_exe().ok()?;
    let run = |scenario: &str| {
        std::process::Command::new(&exe)
            .args(["--perf-scenario", scenario, "--perf-child", "--out"])
            .arg(&opts.out)
            .env("PARLA_DATA_DIR", dir.join("data"))
            .env("PARLA_CACHE_DIR", dir.join("cache"))
            .status()
            .map(|s| s.code().unwrap_or(1))
            .unwrap_or(1)
    };
    let mut code = run("warm");
    if code == 0 {
        if opts.scenario == "start-cold" && !purge_os_file_cache() {
            eprintln!(
                "start-cold: the OS file cache could not be purged (needs `sudo purge`); this run is warm"
            );
        }
        code = run(&opts.scenario);
    }
    let _ = std::fs::remove_dir_all(&dir);
    Some(code)
}

fn purge_os_file_cache() -> bool {
    cfg!(target_os = "macos")
        && std::process::Command::new("sudo")
            .args(["-n", "purge"])
            .status()
            .is_ok_and(|s| s.success())
}

type Window = WindowHandle<RootView>;

struct Env {
    cx: AsyncApp,
    window: Window,
    provider: Arc<FakeProvider>,
}

impl Env {
    async fn sleep(&self, d: Duration) {
        self.cx.background_executor().timer(d).await;
    }

    /// Resolves with the time of the next drawn frame, forcing one. A window
    /// that is hidden, covered or on a sleeping display draws nothing (the
    /// display link is stopped), so waiting without a limit would hang the run
    /// for good: give up after [`FRAME_STALL`] and say why.
    async fn next_frame(&mut self) -> Result<Instant, String> {
        let (tx, rx) = oneshot::channel();
        self.window
            .update(&mut self.cx, |_, window, _| {
                window.on_next_frame(move |_, _| {
                    let _ = tx.send(Instant::now());
                });
                window.refresh();
            })
            .map_err(|_| "window closed".to_string())?;
        let stall = self.cx.background_executor().timer(FRAME_STALL);
        futures::pin_mut!(rx, stall);
        match futures::future::select(rx, stall).await {
            futures::future::Either::Left((frame, _)) => frame.map_err(|_| "window closed".into()),
            futures::future::Either::Right(_) => Err(format!(
                "no frame within {} s: is the window covered, minimized or on a sleeping display?",
                FRAME_STALL.as_secs()
            )),
        }
    }

    /// The session's state through `f`.
    fn read<R>(
        &mut self,
        f: impl FnOnce(&RootView, &crate::session::WorkspaceSession) -> R,
    ) -> Option<R> {
        self.window
            .update(&mut self.cx, |root, _, cx| {
                let s = root.session()?;
                Some(f(root, s.read(cx)))
            })
            .ok()
            .flatten()
    }

    async fn wait_for(
        &mut self,
        what: &str,
        mut done: impl FnMut(&mut Env) -> bool,
    ) -> Result<(), String> {
        for _ in 0..6_000 {
            if done(self) {
                return Ok(());
            }
            self.sleep(Duration::from_millis(5)).await;
        }
        Err(format!("timed out waiting for {what}"))
    }

    async fn ready(&mut self) -> Result<(), String> {
        self.wait_for("the workspace", |e| {
            e.window
                .update(&mut e.cx, |root, _, cx| {
                    *root.phase() == Phase::Ready
                        && root
                            .session()
                            .is_some_and(|s| s.read(cx).state().conversations.len() >= 300)
                })
                .unwrap_or(false)
        })
        .await
    }

    /// Waits until all `n` workspaces are lean (`lean_after_minutes` plus a minute).
    async fn wait_lean(&mut self, n: usize) -> Result<(), String> {
        let limit = self.cx.update(|cx| {
            u64::from(
                cx.global::<settings::SettingsStore>()
                    .current()
                    .performance
                    .lean_after_minutes,
            )
        }) * 60
            + 60;
        for _ in 0..limit {
            let lean = self.cx.update(|cx| {
                let h = cx.try_global::<crate::workspaces::WorkspacesHandle>()?;
                let w = h.0.read(cx);
                let all = w.sessions().count() == n && w.sessions().all(|s| s.read(cx).is_lean());
                Some(all)
            });
            if lean == Some(true) {
                return Ok(());
            }
            self.sleep(Duration::from_secs(1)).await;
        }
        Err("workspaces did not go lean".into())
    }

    async fn open(&mut self, c: &str) -> Result<(), String> {
        let id = ConversationId(Arc::from(c));
        let (id2, id3) = (id.clone(), id.clone());
        self.window
            .update(&mut self.cx, move |root, _, cx| {
                if let Some(s) = root.session() {
                    s.update(cx, |s, cx| s.open(id2, cx));
                }
            })
            .map_err(|e| e.to_string())?;
        self.wait_for("the conversation", move |e| {
            e.read(|root, s| {
                root.shown_conversation() == Some(&id3)
                    && s.state()
                        .windows
                        .get(&id3)
                        .is_some_and(|w| !w.messages().is_empty())
            })
            .unwrap_or(false)
        })
        .await?;
        self.next_frame().await?;
        Ok(())
    }
}

fn posted(conversation: &str, n: u64) -> (ProviderEvent, MessageTs) {
    let ts = MessageTs(Arc::from(format!(
        "{}.{:06}",
        1_900_000_000 + n,
        n % 1_000_000
    )));
    let mut m = Message::new(
        MessageId {
            conversation: ConversationId(Arc::from(conversation)),
            ts: ts.clone(),
        },
        Some(domain::UserId(Arc::from(member_id(1 + (n % 50) as usize)))),
        Arc::from("deploy is *green* <https://example.com/p/1|link> :tada:"),
    );
    m.plain = m.text.clone();
    (ProviderEvent::MessagePosted(m), ts)
}

fn report(opts: &Options, scenario: &str, seconds: f64, metrics: &[(&str, f64)]) -> Report {
    let cmd = |prog: &str, args: &[&str]| {
        std::process::Command::new(prog)
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let _ = opts;
    Report {
        scenario: scenario.into(),
        commit: cmd("git", &["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "unknown".into()),
        os: std::env::consts::OS.into(),
        machine: cmd("sysctl", &["-n", "machdep.cpu.brand_string"])
            .unwrap_or_else(|| std::env::consts::ARCH.into()),
        seconds,
        metrics: metrics
            .iter()
            .filter(|(_, v)| v.is_finite())
            .map(|(k, v)| ((*k).to_string(), *v))
            .collect(),
    }
}

fn cpu_pct(a: Sample, b: Sample, secs: f64) -> f64 {
    (b.cpu_s - a.cpu_s) / secs * 100.0
}

/// Idle windows a run measures at most. Input during a window (a hover, a
/// focus change: a person using the machine) is not idle work, but it costs
/// wakeups: the frame that answers it keeps the display link ticking through
/// its idle grace, about 60 wakeups at 120 Hz, which is 1/s over a 60 s
/// window. Such a window is measured again.
const IDLE_ATTEMPTS: usize = 3;

/// Input-free time before an idle window starts: covers the display link's
/// idle grace and the window server's follow-up (occlusion, key window) after
/// the last input.
const IDLE_SETTLE: Duration = Duration::from_secs(2);

/// Settle periods that may see input before the window starts anyway.
const IDLE_SETTLE_TRIES: usize = 10;

/// Whether the idle scenario measures another window, given the input events
/// each window so far saw: only after input, and at most `attempts` windows.
fn measure_idle_again(inputs: &[u64], attempts: usize) -> bool {
    inputs.len() < attempts && inputs.last().is_some_and(|&n| n > 0)
}

/// The window the idle scenario reports: the first one without input, else
/// the one with the least.
fn pick_idle_window(inputs: &[u64]) -> usize {
    inputs.iter().position(|&n| n == 0).unwrap_or_else(|| {
        (0..inputs.len())
            .min_by_key(|&i| (inputs[i], i))
            .unwrap_or(0)
    })
}

/// One measured idle window.
struct IdleWindow {
    a: Sample,
    b: Sample,
    dt: f64,
    frames: u64,
    input_frames: u64,
    inputs: u64,
}

async fn idle(env: &mut Env, opts: &Options, hidden: bool) -> Result<Report, String> {
    env.open(&conversation_id(0)).await?;
    if hidden {
        env.window
            .update(&mut env.cx, |_, window, _| window.minimize_window())
            .map_err(|e| e.to_string())?;
    }
    if hidden && opts.workspaces > 1 {
        // The row measures lean workspaces: wait for every timer to fire.
        env.wait_lean(opts.workspaces).await?;
    }
    // A focus change is input even when it draws no frame (a covered window).
    let activations = Rc::new(Cell::new(0u64));
    let _activation = {
        let seen = activations.clone();
        env.window
            .update(&mut env.cx, move |_, window, cx| {
                cx.observe_window_activation(window, move |_, _, _| seen.set(seen.get() + 1))
            })
            .map_err(|e| e.to_string())?
    };
    let inputs = || input_frames() + activations.get();
    let mut windows: Vec<IdleWindow> = Vec::new();
    loop {
        for _ in 0..IDLE_SETTLE_TRIES {
            let before = inputs();
            env.sleep(IDLE_SETTLE).await;
            if inputs() == before {
                break;
            }
        }
        let (a, f0, i0, n0, t0) = (
            sample().ok_or("no process sampler on this OS")?,
            frames_drawn(),
            input_frames(),
            inputs(),
            Instant::now(),
        );
        env.sleep(Duration::from_secs(opts.seconds)).await;
        let (b, f1, i1, n1, dt) = (
            sample().ok_or("sampler")?,
            frames_drawn(),
            input_frames(),
            inputs(),
            t0.elapsed().as_secs_f64(),
        );
        windows.push(IdleWindow {
            a,
            b,
            dt,
            frames: f1 - f0,
            input_frames: i1 - i0,
            inputs: n1 - n0,
        });
        let seen: Vec<u64> = windows.iter().map(|w| w.inputs).collect();
        if !measure_idle_again(&seen, IDLE_ATTEMPTS) {
            break;
        }
        eprintln!(
            "{}: input during the window ({} events), measuring again",
            opts.scenario,
            n1 - n0
        );
    }
    let seen: Vec<u64> = windows.iter().map(|w| w.inputs).collect();
    let w = &windows[pick_idle_window(&seen)];
    Ok(report(
        opts,
        &opts.scenario,
        w.dt,
        &[
            ("mem_mb", w.b.memory_mb),
            ("cpu_pct", cpu_pct(w.a, w.b, w.dt)),
            ("wakeups_per_s", (w.b.wakeups - w.a.wakeups) / w.dt),
            // Frames that answer a person using the machine during the run (a
            // hover, a focus change) are reported apart: they are not idle work.
            ("frames", (w.frames - w.input_frames) as f64),
            ("frames_after_input", w.input_frames as f64),
        ],
    ))
}

/// Animated emoji: 30 animated custom emoji (three messages of ten) visible in
/// the shown conversation. `hidden` minimizes the window first, so animation
/// must stop (zero frames, idle CPU).
async fn emoji(env: &mut Env, opts: &Options, hidden: bool) -> Result<Report, String> {
    env.open(&conversation_id(0)).await?;
    for n in 0..3u64 {
        let (mut ev, _) = posted(&conversation_id(0), 900 + n);
        if let ProviderEvent::MessagePosted(m) = &mut ev {
            m.text = Arc::from(":party: ".repeat(10).trim_end());
            m.plain = m.text.clone();
        }
        env.provider.push_event(ev);
    }
    // Let the messages arrive, the GIFs load and start playing.
    env.sleep(Duration::from_secs(3)).await;
    if hidden {
        env.window
            .update(&mut env.cx, |_, window, _| window.minimize_window())
            .map_err(|e| e.to_string())?;
        env.sleep(Duration::from_secs(2)).await;
    }
    let (a, f0, t0) = (
        sample().ok_or("no process sampler on this OS")?,
        frames_drawn(),
        Instant::now(),
    );
    env.sleep(Duration::from_secs(opts.seconds)).await;
    let (b, f1, dt) = (
        sample().ok_or("sampler")?,
        frames_drawn(),
        t0.elapsed().as_secs_f64(),
    );
    Ok(report(
        opts,
        &opts.scenario,
        dt,
        &[
            ("mem_mb", b.memory_mb),
            ("cpu_pct", cpu_pct(a, b, dt)),
            ("wakeups_per_s", (b.wakeups - a.wakeups) / dt),
            ("frames_per_s", (f1 - f0) as f64 / dt),
        ],
    ))
}

/// Draw-time and key-to-present p95/p99 in ms from GPUI's frame profiler.
/// The profiler (`gpui/profiler`) sits behind the `frame-profiler`
/// feature (it adds memory for its histograms); without it the frame metrics are absent from the report and
/// `perf_check` skips their limits.
#[cfg(feature = "frame-profiler")]
fn draw_percentiles(window: &Window, cx: &mut AsyncApp, key_to_present: bool) -> (f64, f64) {
    window
        .update(cx, |_, window, _| {
            let s = window.frame_duration_snapshot();
            let h = if key_to_present {
                &s.dirty_to_present_histogram
            } else {
                &s.draw_duration_histogram
            };
            (
                h.value_at_percentile(95.0) as f64 / 1e6,
                h.value_at_percentile(99.0) as f64 / 1e6,
            )
        })
        .unwrap_or((f64::NAN, f64::NAN))
}

#[cfg(not(feature = "frame-profiler"))]
fn draw_percentiles(_: &Window, _: &mut AsyncApp, _: bool) -> (f64, f64) {
    (f64::NAN, f64::NAN)
}

/// Clears the profiler's frame statistics before a measured run.
fn reset_frame_stats(window: &mut gpui::Window) {
    #[cfg(feature = "frame-profiler")]
    window.reset_debug_frame_overlay_stats();
    #[cfg(not(feature = "frame-profiler"))]
    let _ = window;
}

/// Re-arms a next-frame callback on every frame while `alive` is set, so the
/// platform keeps delivering frame requests.
fn keep_frames_coming(window: &mut gpui::Window, alive: Rc<Cell<bool>>) {
    window.on_next_frame(move |window, _| {
        if alive.get() {
            keep_frames_coming(window, alive);
        }
    });
}

async fn scroll(env: &mut Env, opts: &Options) -> Result<Report, String> {
    env.open(&conversation_id(0)).await?;
    env.window
        .update(&mut env.cx, |_, window, _| reset_frame_stats(window))
        .map_err(|e| e.to_string())?;
    let (a, t0) = (
        sample().ok_or("no process sampler on this OS")?,
        Instant::now(),
    );
    let mut intervals = Vec::new();
    let mut last = Instant::now();
    let mut up = true;
    // A fling requests a frame after every frame. On macOS GPUI parks the
    // display link when a frame leaves nothing pending, and restarting it for
    // the next scroll event costs up to a vsync, which would count as a
    // dropped frame although the app drew in time. Keep the link running for
    // the measured run, as momentum scrolling does.
    let alive = Rc::new(Cell::new(true));
    {
        let alive = alive.clone();
        env.window
            .update(&mut env.cx, |_, window, _| {
                keep_frames_coming(window, alive)
            })
            .map_err(|e| e.to_string())?;
    }
    while t0.elapsed() < Duration::from_secs(opts.seconds) {
        let dy = if up { 100.0 } else { -100.0 };
        let r = env.window.update(&mut env.cx, |_, window, cx| {
            window.dispatch_event(
                PlatformInput::Scroll(ScrollEvent {
                    // design: allow synthetic scroll position inside the perf harness window, not UI chrome
                    position: point(px(700.0), px(300.0)),
                    delta: ScrollDelta::Pixels(point(px(0.0), px(dy))),
                    modifiers: Modifiers::default(),
                    touch_phase: TouchPhase::Moved,
                    source: ScrollSource::Pointer,
                }),
                cx,
            );
        });
        r.map_err(|e| e.to_string())?;
        let t = env.next_frame().await?;
        intervals.push(t.duration_since(last).as_secs_f64() * 1e3);
        last = t;
        // Fling up through the history, then back down, for the whole run.
        if intervals.len() % 600 == 0 {
            up = !up;
        }
    }
    alive.set(false);
    let (b, dt) = (sample().ok_or("sampler")?, t0.elapsed().as_secs_f64());
    let (p95, p99) = draw_percentiles(&env.window, &mut env.cx, false);
    Ok(report(
        opts,
        "scroll",
        dt,
        &[
            ("mem_mb", b.memory_mb),
            ("cpu_pct", cpu_pct(a, b, dt)),
            ("frame_p95_ms", p95),
            ("frame_p99_ms", p99),
            ("dropped_pct", dropped_pct(&intervals, refresh_ms())),
        ],
    ))
}

/// Scroll events at about 120 Hz with no armed next-frame callback: the only
/// demand for a frame is the event itself, as with a real trackpad. A frame
/// that leaves nothing pending parks the display link on macOS, so this
/// measures what a user sees when the link has to be running (or restarted)
/// for each event. Reports the interval between presented frames.
async fn scroll_input(env: &mut Env, opts: &Options) -> Result<Report, String> {
    env.open(&conversation_id(0)).await?;
    env.window
        .update(&mut env.cx, |_, window, _| reset_frame_stats(window))
        .map_err(|e| e.to_string())?;
    let (a, t0) = (
        sample().ok_or("no process sampler on this OS")?,
        Instant::now(),
    );
    // For each event: when it was sent and when the frame it caused started.
    let frames: Rc<RefCell<Vec<(Instant, Instant)>>> = Rc::default();
    let mut up = true;
    let mut sent = 0u64;
    while t0.elapsed() < Duration::from_secs(opts.seconds) {
        let dy = if up { 100.0 } else { -100.0 };
        let frames = frames.clone();
        let at = Instant::now();
        env.window
            .update(&mut env.cx, |_, window, cx| {
                // Observes the frame this event causes; no `refresh`.
                window.on_next_frame(move |_, _| frames.borrow_mut().push((at, Instant::now())));
                window.dispatch_event(
                    PlatformInput::Scroll(ScrollEvent {
                        // design: allow synthetic scroll position inside the perf harness window, not UI chrome
                        position: point(px(700.0), px(300.0)),
                        delta: ScrollDelta::Pixels(point(px(0.0), px(dy))),
                        modifiers: Modifiers::default(),
                        touch_phase: TouchPhase::Moved,
                        source: ScrollSource::Pointer,
                    }),
                    cx,
                );
            })
            .map_err(|e| e.to_string())?;
        sent += 1;
        if sent.is_multiple_of(600) {
            up = !up;
        }
        env.sleep(Duration::from_micros(8333)).await;
    }
    let (b, dt) = (sample().ok_or("sampler")?, t0.elapsed().as_secs_f64());
    let frames = frames.borrow();
    let ms = |d: Duration| d.as_secs_f64() * 1e3;
    let latencies: Vec<f64> = frames
        .iter()
        .map(|(s, f)| ms(f.duration_since(*s)))
        .collect();
    let gaps: Vec<f64> = frames
        .windows(2)
        .map(|w| ms(w[1].0.duration_since(w[0].0)))
        .collect();
    let intervals: Vec<f64> = frames
        .windows(2)
        .map(|w| ms(w[1].1.duration_since(w[0].1)))
        // Several events can arm callbacks for the same frame: count it once.
        .filter(|ms| *ms > 2.0)
        .collect();
    let drawn = intervals.len() + 1;
    let refresh = refresh_ms();
    let (p95, p99) = draw_percentiles(&env.window, &mut env.cx, false);
    Ok(report(
        opts,
        "scroll-input",
        dt,
        &[
            ("mem_mb", b.memory_mb),
            ("cpu_pct", cpu_pct(a, b, dt)),
            ("frame_p95_ms", p95),
            ("frame_p99_ms", p99),
            // The frame cadence follows the input cadence (the timer that
            // sends events runs slower than 120 Hz), so these describe the
            // run; `dropped_pct` judges each event against the next vsync.
            ("refresh_hz", refresh.map_or(f64::NAN, |r| 1000.0 / r)),
            ("input_gap_p50_ms", percentile(&gaps, 50.0)),
            ("frames_per_s", drawn as f64 / dt),
            ("interval_p95_ms", percentile(&intervals, 95.0)),
            ("event_to_frame_p99_ms", percentile(&latencies, 99.0)),
            ("cpu_ms_per_frame", (b.cpu_s - a.cpu_s) * 1e3 / drawn as f64),
            ("dropped_pct", late_pct(&latencies, refresh)),
        ],
    ))
}

async fn typing(env: &mut Env, opts: &Options) -> Result<Report, String> {
    env.open(&conversation_id(0)).await?;
    env.window
        .update(&mut env.cx, |root, window, cx| {
            root.perf_prepare_composer(&"x".repeat(500), window, cx)
        })
        .map_err(|e| e.to_string())?;
    env.next_frame().await?;
    env.window
        .update(&mut env.cx, |_, window, _| reset_frame_stats(window))
        .map_err(|e| e.to_string())?;
    let (a, t0) = (
        sample().ok_or("no process sampler on this OS")?,
        Instant::now(),
    );
    for i in 0..(opts.seconds * 10) {
        let key = char::from(b'a' + (i % 26) as u8).to_string();
        // Not `WindowHandle::update`: key handlers update the root view, which
        // that call would hold leased.
        env.cx
            .update_window(env.window.into(), |_, window, cx| {
                if let Ok(k) = Keystroke::parse_with_os(&key, cx.platform_info().os) {
                    window.dispatch_keystroke(k, cx);
                }
            })
            .map_err(|e| e.to_string())?;
        env.sleep(Duration::from_millis(100)).await;
    }
    let (b, dt) = (sample().ok_or("sampler")?, t0.elapsed().as_secs_f64());
    let (p95, p99) = draw_percentiles(&env.window, &mut env.cx, true);
    Ok(report(
        opts,
        "type",
        dt,
        &[
            ("mem_mb", b.memory_mb),
            ("cpu_pct", cpu_pct(a, b, dt)),
            ("frame_p95_ms", p95),
            ("frame_p99_ms", p99),
        ],
    ))
}

async fn switch(env: &mut Env, opts: &Options) -> Result<Report, String> {
    let convs: Vec<String> = (0..8).map(conversation_id).collect();
    for c in &convs {
        env.open(c).await?;
    }
    let mut switch_ms = Vec::new();
    for round in 0..40usize {
        let c = convs[round % convs.len()].clone();
        let id = ConversationId(Arc::from(c.as_str()));
        let t0 = Instant::now();
        let id2 = id.clone();
        env.window
            .update(&mut env.cx, move |root, _, cx| {
                if let Some(s) = root.session() {
                    s.update(cx, |s, cx| s.open(id2, cx));
                }
            })
            .map_err(|e| e.to_string())?;
        loop {
            let t = env.next_frame().await?;
            let shown = env
                .read(|root, _| root.shown_conversation() == Some(&id))
                .unwrap_or(false);
            if shown {
                switch_ms.push(t.duration_since(t0).as_secs_f64() * 1e3);
                break;
            }
        }
    }
    // A live message to the shown conversation, socket frame to painted row.
    let shown = conversation_id(7);
    let mut event_ms = Vec::new();
    for n in 0..40u64 {
        let (ev, ts) = posted(&shown, n);
        let t0 = Instant::now();
        env.provider.push_event(ev);
        loop {
            let t = env.next_frame().await?;
            let seen = env
                .read(|_, s| {
                    s.state()
                        .windows
                        .get(&ConversationId(Arc::from(shown.as_str())))
                        .and_then(|w| w.newest_ts().cloned())
                        == Some(ts.clone())
                })
                .unwrap_or(false);
            if seen {
                event_ms.push(t.duration_since(t0).as_secs_f64() * 1e3);
                break;
            }
        }
    }
    let b = sample().ok_or("no process sampler on this OS")?;
    let _ = opts;
    Ok(report(
        opts,
        "switch",
        0.0,
        &[
            ("mem_mb", b.memory_mb),
            ("latency_ms", percentile(&switch_ms, 95.0)),
            ("event_latency_ms", percentile(&event_ms, 95.0)),
        ],
    ))
}

async fn start(env: &mut Env, opts: &Options) -> Result<Report, String> {
    let t0 = process_start();
    let paint = crate::perf::first_frame()
        .map(|t| t.duration_since(t0).as_secs_f64() * 1e3)
        .ok_or("no frame drawn yet")?;
    env.wait_for("cached content", |e| {
        e.read(|_, s| s.state().windows.values().any(|w| !w.messages().is_empty()))
            .unwrap_or(false)
    })
    .await?;
    let content = env
        .next_frame()
        .await
        .map(|t| t.duration_since(t0).as_secs_f64() * 1e3)?;
    let b = sample().ok_or("no process sampler on this OS")?;
    Ok(report(
        opts,
        &opts.scenario,
        content / 1e3,
        &[
            ("mem_mb", b.memory_mb),
            ("paint_ms", paint),
            ("content_ms", content),
        ],
    ))
}

async fn warm(env: &mut Env, opts: &Options) -> Result<Report, String> {
    env.open(&conversation_id(0)).await?;
    env.sleep(Duration::from_secs(3)).await;
    Ok(report(opts, "warm", 3.0, &[]))
}

async fn soak(env: &mut Env, opts: &Options) -> Result<Report, String> {
    env.open(&conversation_id(0)).await?;
    let total = (opts.hours * 3600.0) as u64;
    let mark_at = total.min(3600).max(total / 8);
    let mut mark = 0.0;
    let t0 = Instant::now();
    for sec in 0..total {
        if sec % 120 == 0 {
            env.open(&conversation_id(((sec / 120) % 8) as usize))
                .await?;
        }
        let shown = conversation_id(((sec / 120) % 8) as usize);
        env.provider.push_event(posted(&shown, sec).0);
        env.sleep(Duration::from_secs(1)).await;
        if sec == mark_at {
            mark = sample().ok_or("sampler")?.memory_mb;
        }
    }
    env.sleep(Duration::from_secs(600)).await;
    let b = sample().ok_or("no process sampler on this OS")?;
    Ok(report(
        opts,
        "soak",
        t0.elapsed().as_secs_f64(),
        &[
            ("mem_mb", b.memory_mb),
            ("mem_drift_mb", b.memory_mb - mark),
        ],
    ))
}

/// Runs the scenario once the window exists, writes the report and quits.
pub fn spawn(opts: Options, window: Window, provider: Arc<FakeProvider>, cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut env = Env {
            cx: cx.clone(),
            window,
            provider,
        };
        let result = async {
            env.ready().await?;
            match opts.scenario.as_str() {
                "idle" => idle(&mut env, &opts, false).await,
                "idle-hidden" => idle(&mut env, &opts, true).await,
                "emoji" => emoji(&mut env, &opts, false).await,
                "emoji-hidden" => emoji(&mut env, &opts, true).await,
                "scroll" => scroll(&mut env, &opts).await,
                "scroll-input" => scroll_input(&mut env, &opts).await,
                "type" => typing(&mut env, &opts).await,
                "switch" => switch(&mut env, &opts).await,
                "start" | "start-cold" => start(&mut env, &opts).await,
                "warm" => warm(&mut env, &opts).await,
                "soak" => soak(&mut env, &opts).await,
                other => Err(format!("unknown scenario {other}; one of {SCENARIOS:?}")),
            }
        }
        .await;
        let code = match result {
            Ok(r) => {
                if r.scenario != "warm" {
                    match r.write(&opts.out) {
                        Ok(p) => println!("{}: {:?} -> {}", r.scenario, r.metrics, p.display()),
                        Err(e) => eprintln!("cannot write the report: {e}"),
                    }
                }
                0
            }
            Err(e) => {
                eprintln!("perf scenario {} failed: {e}", opts.scenario);
                1
            }
        };
        cx.update(|cx| cx.quit());
        // `quit` returns before the event loop ends; leave with the right status.
        std::thread::sleep(Duration::from_millis(300));
        std::process::exit(code);
    })
    .detach();
}

/// The main display's refresh period in ms, `None` where the OS does not say.
#[cfg(target_os = "macos")]
fn refresh_ms() -> Option<f64> {
    let mtm = objc2_foundation::MainThreadMarker::new()?;
    let fps = objc2_app_kit::NSScreen::mainScreen(mtm)?.maximumFramesPerSecond();
    (fps > 0).then(|| 1000.0 / fps as f64)
}

#[cfg(not(target_os = "macos"))]
fn refresh_ms() -> Option<f64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_reports_the_first_window_without_input() {
        assert_eq!(pick_idle_window(&[0]), 0);
        assert_eq!(pick_idle_window(&[2, 0, 0]), 1);
        assert_eq!(pick_idle_window(&[1, 3, 0]), 2);
    }

    #[test]
    fn idle_with_input_in_every_window_reports_the_least_disturbed() {
        assert_eq!(pick_idle_window(&[3, 1, 2]), 1);
        assert_eq!(pick_idle_window(&[2, 2]), 0);
    }

    #[test]
    fn idle_measures_again_only_after_input() {
        assert!(!measure_idle_again(&[0], IDLE_ATTEMPTS));
        assert!(measure_idle_again(&[1], IDLE_ATTEMPTS));
        assert!(measure_idle_again(&[1, 4], IDLE_ATTEMPTS));
        assert!(!measure_idle_again(&[1, 4, 2], IDLE_ATTEMPTS));
        assert!(!measure_idle_again(&[1, 0], IDLE_ATTEMPTS));
    }
}

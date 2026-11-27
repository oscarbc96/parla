//! `call.native` measurement: joins a real huddle
//! with `calls::native::NativeCall` and real cpal devices, samples the process
//! once a second, leaves, and writes `<out>/call-native.json` in the perf
//! report format (`mise run perf:check` reads it).
//!
//! Skipped unless `PARLA_PERF_NATIVE_CALL=1`: it needs the owner's credentials
//! and live network, so CI never runs it. `mise run perf:call-native`.
//!
//! Environment: `PARLA_PERF_NATIVE_TEAM`, `PARLA_PERF_NATIVE_CHANNEL` (ids of the
//! test workspace and channel), optional `PARLA_PERF_NATIVE_SECONDS` (default
//! 300) and `PARLA_PERF_NATIVE_MIC=1` (open the microphone; without it the call
//! runs receive-only so no OS permission prompt can appear). Credentials come
//! from `<data_dir>/development_credentials.json` (debug builds' store) and
//! are never printed. Output is counts, timings and booleans only.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use calls::native::{
    CallEvent, CpalSink, CpalSource, Devices, NativeCall, PipelineError, SourceFactory,
};
use domain::{NativeCallJoin, SecretText};
use futures::StreamExt;
use parla::perf::report::{Report, percentile, sample};
use settings::WorkspaceKey;
use slack_api::{ClientConfig, WebClient};

/// The `perf_budget.toml` limit for the media thread and its buffers to be gone.
const LEAVE_LIMIT_MS: f64 = 2000.0;
/// Give up if media is not live this long after joining.
const LIVE_TIMEOUT: Duration = Duration::from_secs(30);

/// Replaces long token-like runs so an error text is safe to print.
fn mask(s: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        out.push_str(if run.len() >= 16 { "<masked>" } else { run });
        run.clear();
    };
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '=') {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    out
}

/// OS threads of this process (the media and call threads show up here).
fn thread_count() -> usize {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir("/proc/self/task").map_or(0, Iterator::count)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let out = std::process::Command::new("ps")
            .args(["-M", "-p", &std::process::id().to_string()])
            .output();
        // One header line, then one line per thread.
        out.map_or(0, |o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .count()
                .saturating_sub(1)
        })
    }
}

/// Threads of this process whose name starts with `parla-` (the call engine
/// `parla-call` and the pipeline's `parla-media`): none may outlive a leave.
fn call_threads() -> usize {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_dir("/proc/self/task")
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|t| std::fs::read_to_string(t.path().join("comm")).ok())
            .filter(|n| n.starts_with("parla-"))
            .count()
    }
    #[cfg(target_os = "macos")]
    {
        // `proc_pidinfo` flavors from <sys/proc_info.h>.
        const PROC_PIDLISTTHREADS: i32 = 6;
        const PROC_PIDTHREADID64INFO: i32 = 15;
        let pid = std::process::id() as i32;
        let mut ids = vec![0u64; 1024];
        // SAFETY: the buffer is `ids.len() * 8` bytes, as passed.
        let bytes = unsafe {
            libc::proc_pidinfo(
                pid,
                PROC_PIDLISTTHREADS,
                0,
                ids.as_mut_ptr().cast(),
                (ids.len() * 8) as i32,
            )
        };
        let n = usize::try_from(bytes).unwrap_or(0) / 8;
        ids.iter()
            .take(n)
            .filter(|id| {
                // SAFETY: `info` is plain data of the size passed.
                let mut info: libc::proc_threadinfo = unsafe { std::mem::zeroed() };
                let got = unsafe {
                    libc::proc_pidinfo(
                        pid,
                        PROC_PIDTHREADID64INFO,
                        **id,
                        (&raw mut info).cast(),
                        std::mem::size_of::<libc::proc_threadinfo>() as i32,
                    )
                };
                let name: Vec<u8> = info
                    .pth_name
                    .iter()
                    .take_while(|c| **c != 0)
                    .map(|c| *c as u8)
                    .collect();
                got > 0 && name.starts_with(b"parla-")
            })
            .count()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        0
    }
}

fn load_join(team: &str, channel: &str) -> Result<NativeCallJoin, String> {
    let path = settings::paths::data_dir().join("development_credentials.json");
    let raw = std::fs::read(&path).map_err(|_| "credentials file unreadable".to_string())?;
    let json: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|_| "credentials file is not json".to_string())?;
    let entry = json
        .get(format!("parla://workspace/{team}"))
        .ok_or("no credentials entry for the team")?;
    let user = entry["user"].as_str().unwrap_or("");
    let value = entry["value"].as_str().unwrap_or("");
    let ws = WorkspaceKey::new("slack", team);
    let creds = slack_auth::decode_credentials(&ws, user, value.as_bytes())
        .map_err(|_| "credentials did not decode".to_string())?;
    let client = WebClient::new(
        ClientConfig::for_workspace(&creds.url),
        creds.token,
        creds.cookie,
    )
    .map_err(|e| format!("client: {}", mask(&e.to_string())))?;
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let j = rt
        .block_on(client.rooms_join(channel))
        .map_err(|e| format!("rooms.join: {}", mask(&e.to_string())))?;
    Ok(NativeCallJoin {
        meeting_id: Arc::from(j.meeting_id),
        attendee_id: Arc::from(j.attendee_id),
        external_user_id: Arc::from(j.external_user_id),
        join_token: SecretText::new(j.join_token.expose()),
        signaling_url: Arc::from(j.signaling_url),
        audio_host_url: Arc::from(j.audio_host_url),
        turn_control_url: j.turn_control_url.map(Arc::from),
    })
}

fn run() -> Result<(), String> {
    let env = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
    let team = env("PARLA_PERF_NATIVE_TEAM").ok_or("set PARLA_PERF_NATIVE_TEAM")?;
    let channel = env("PARLA_PERF_NATIVE_CHANNEL").ok_or("set PARLA_PERF_NATIVE_CHANNEL")?;
    let seconds: u64 = env("PARLA_PERF_NATIVE_SECONDS")
        .and_then(|s| s.parse().ok())
        .unwrap_or(300);
    let mic = env("PARLA_PERF_NATIVE_MIC").as_deref() == Some("1");

    // Warm the process (HTTP stack, allocator, runtimes) before the baseline,
    // so the delta is the call's own cost.
    let join = load_join(&team, &channel)?;
    std::thread::sleep(Duration::from_secs(2));
    let base = sample().ok_or("no process counters on this OS")?;
    let threads_before = thread_count();

    let source: SourceFactory = if mic {
        CpalSource::factory()
    } else {
        Box::new(|| Err(PipelineError::Device("microphone not requested".into())))
    };
    let devices = Devices {
        source,
        sink: CpalSink::factory(),
    };
    let (call, mut events) = NativeCall::join(join, devices);

    let live = Arc::new(AtomicBool::new(false));
    let ended = Arc::new(AtomicBool::new(false));
    let capture_unavailable = Arc::new(AtomicBool::new(false));
    let peers = Arc::new(AtomicUsize::new(0));
    let (l, e, c, p) = (
        live.clone(),
        ended.clone(),
        capture_unavailable.clone(),
        peers.clone(),
    );
    let drain = std::thread::spawn(move || {
        futures::executor::block_on(async move {
            while let Some(ev) = events.next().await {
                match ev {
                    CallEvent::Live => l.store(true, Ordering::Relaxed),
                    CallEvent::CaptureUnavailable(_) => c.store(true, Ordering::Relaxed),
                    CallEvent::Participants(v) => p.store(v.len(), Ordering::Relaxed),
                    CallEvent::Failed {
                        before_media,
                        reason,
                    } => {
                        eprintln!(
                            "call failed (before media: {before_media}): {}",
                            mask(&reason)
                        );
                        e.store(true, Ordering::Relaxed);
                    }
                    CallEvent::Left => e.store(true, Ordering::Relaxed),
                    _ => {}
                }
            }
            e.store(true, Ordering::Relaxed);
        });
    });

    let t0 = Instant::now();
    while !live.load(Ordering::Relaxed) {
        if ended.load(Ordering::Relaxed) || t0.elapsed() > LIVE_TIMEOUT {
            call.leave();
            let _ = drain.join();
            return Err("the call never went live".into());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    eprintln!(
        "live after {:.1} s; sampling {seconds} s",
        t0.elapsed().as_secs_f64()
    );

    // One reading a second for the timed part.
    let start = sample().ok_or("no process counters")?;
    let started = Instant::now();
    let mut last = start;
    let mut last_at = started;
    let mut cpu_pcts: Vec<f64> = Vec::new();
    let mut peak_mem = start.memory_mb;
    let mut mem_sum = 0.0;
    let mut n = 0.0_f64;
    while started.elapsed() < Duration::from_secs(seconds) {
        std::thread::sleep(Duration::from_secs(1));
        let Some(s) = sample() else { continue };
        let now = Instant::now();
        let dt = now.duration_since(last_at).as_secs_f64();
        if dt > 0.0 {
            cpu_pcts.push((s.cpu_s - last.cpu_s) / dt * 100.0);
        }
        peak_mem = peak_mem.max(s.memory_mb);
        mem_sum += s.memory_mb;
        n += 1.0;
        last = s;
        last_at = now;
        if ended.load(Ordering::Relaxed) {
            return Err("the call ended during the measurement".into());
        }
    }
    let wall = started.elapsed().as_secs_f64();
    let cpu_mean = (last.cpu_s - start.cpu_s) / wall * 100.0;
    let participants = peers.load(Ordering::Relaxed);

    // Leave: time until both call threads are gone, then until the process is
    // back to its pre-call thread count and memory.
    let left_at = Instant::now();
    call.leave();
    let leave_ms = left_at.elapsed().as_secs_f64() * 1000.0;
    let _ = drain.join();
    let threads_after = thread_count();
    let call_threads_after = call_threads();
    std::thread::sleep(Duration::from_secs(2));
    let after = sample().ok_or("no process counters")?;

    let metrics = [
        ("mem_delta_mb", peak_mem - base.memory_mb),
        ("mem_mean_delta_mb", mem_sum / n.max(1.0) - base.memory_mb),
        ("mem_after_leave_delta_mb", after.memory_mb - base.memory_mb),
        ("cpu_pct", cpu_mean),
        ("cpu_p95_pct", percentile(&cpu_pcts, 95.0)),
        ("leave_ms", leave_ms),
        ("call_threads_left", call_threads_after as f64),
        ("threads_left", threads_after as f64 - threads_before as f64),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), (v * 100.0).round() / 100.0))
    .collect();
    let report = Report {
        scenario: "call-native".into(),
        commit: std::env::var("PARLA_PERF_COMMIT").unwrap_or_default(),
        os: std::env::consts::OS.into(),
        machine: std::env::var("PARLA_PERF_MACHINE").unwrap_or_default(),
        seconds: wall,
        metrics,
    };
    let out = std::env::var_os("PARLA_PERF_OUT")
        .map_or_else(|| PathBuf::from("target/perf"), PathBuf::from);
    let path = report.write(&out).map_err(|e| e.to_string())?;
    println!(
        "participants {participants}; capture {}; report {}",
        if capture_unavailable.load(Ordering::Relaxed) || !mic {
            "receive-only"
        } else {
            "microphone"
        },
        path.display()
    );
    for (k, v) in &report.metrics {
        println!("{k} {v}");
    }
    if call_threads_after > 0 {
        return Err(format!(
            "{call_threads_after} call thread(s) outlived the leave"
        ));
    }
    if leave_ms > LEAVE_LIMIT_MS {
        return Err(format!(
            "leave took {leave_ms:.0} ms, over {LEAVE_LIMIT_MS:.0} ms"
        ));
    }
    Ok(())
}

fn main() {
    if std::env::var("PARLA_PERF_NATIVE_CALL").as_deref() != Ok("1") {
        println!(
            "call-native skipped: set PARLA_PERF_NATIVE_CALL=1 (live network, owner credentials)"
        );
        return;
    }
    if let Err(e) = run() {
        eprintln!("call-native failed: {e}");
        std::process::exit(1);
    }
}

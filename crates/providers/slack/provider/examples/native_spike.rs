//! Native huddle media spike: join the owner's test huddle with `str0m` as TWO
//! attendees of the same user. Attendee A sends 20 s of a 440 Hz Opus tone; attendee B sends
//! silence, counts and decodes what it receives and measures it (RMS, dominant frequency).
//! Prints only booleans, lengths, counts and timings; never a token, id, cookie or URL with an
//! id. Owner-run, joins a real huddle.
//!
//! `cargo run -p slack --example native_spike -- <team id> <channel id>`
//! (credentials from `<data_dir>/development_credentials.json`, debug builds only).

use std::net::SocketAddr;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use chime_signal::{Config, Event, Input, Machine, Output};
use futures::{SinkExt, StreamExt};
use settings::WorkspaceKey;
use slack_api::{ClientConfig, WebClient};
use str0m::change::{SdpAnswer, SdpPendingOffer};
use str0m::media::{Direction, Frequency, MediaKind, MediaTime, Mid};
use str0m::net::{Protocol, Receive};
use str0m::{Candidate, Event as RtcEvent, Input as RtcInput, Output as RtcOutput, Rtc};
use tokio::net::UdpSocket;
use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest, http::HeaderValue};

const SEND_FOR: Duration = Duration::from_secs(20);
const FRAME: Duration = Duration::from_millis(20);
/// Give up if media is not connected this long after the answer is applied.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

/// Replaces every long token-like run (ids, tokens, hashes) so an error text is safe to print.
fn mask(s: &str) -> String {
    let mut out = String::new();
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() >= 16 {
            out.push_str("<masked>");
        } else {
            out.push_str(run);
        }
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

/// An SDP with credentials, fingerprints, ids and addresses removed (shape only).
fn sdp_shape(sdp: &str) -> String {
    let mut out = Vec::new();
    let mut candidates = 0;
    for l in sdp.lines() {
        let l = l.trim_end();
        if l.starts_with("a=candidate") {
            candidates += 1;
            continue;
        }
        let keep = !(l.starts_with("a=ice-ufrag")
            || l.starts_with("a=ice-pwd")
            || l.starts_with("a=fingerprint")
            || l.starts_with("a=msid")
            || l.starts_with("a=ssrc")
            || l.starts_with("o=")
            || l.starts_with("c="));
        if keep {
            out.push(mask(l));
        } else {
            out.push(format!(
                "{}<removed>",
                l.split([':', '=']).next().unwrap_or("")
            ));
        }
    }
    out.push(format!("(a=candidate lines: {candidates})"));
    out.join("\n")
}

/// RFC 7983 demultiplexing of a datagram by its first bytes.
fn class(d: &[u8]) -> &'static str {
    match d.first().copied() {
        Some(0..=3) => "stun",
        Some(20..=63) => "dtls",
        Some(128..=191) if matches!(d.get(1), Some(192..=223)) => "rtcp",
        Some(128..=191) => "rtp",
        _ => "other",
    }
}

/// The datagram class, with the RTCP packet type (200 SR, 201 RR, 205 RTPFB, 206 PSFB) appended.
fn class_detail(d: &[u8]) -> String {
    match (class(d), d.get(1)) {
        ("rtcp", Some(t)) => format!("rtcp-{t}"),
        (c, _) => c.to_string(),
    }
}
fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split(['/', '?']).next().unwrap_or("");
    mask(host)
}

fn step(t0: Instant, name: &str, detail: &str) {
    let role = ROLE.try_with(|r| *r).unwrap_or("-");
    println!(
        "[{:>7.3}s] {role} {name}: {detail}",
        t0.elapsed().as_secs_f64()
    );
}

tokio::task_local! {
    static ROLE: &'static str;
}

#[derive(Clone, Copy, PartialEq)]
enum Role {
    /// Sends a 440 Hz tone.
    Tone,
    /// Sends silence, decodes and measures what it receives.
    Listen,
}

#[derive(Default)]
struct Counts {
    decoder: Option<opus::Decoder>,
    pcm: Vec<f32>,
    sent: u64,
    recv: u64,
    egress_packets: u64,
    ingress_packets: u64,
    out_by_class: std::collections::BTreeMap<String, u64>,
    in_by_class: std::collections::BTreeMap<String, u64>,
}

/// Drains `poll_output` to the next timeout (the single-mutation invariant).
#[allow(clippy::too_many_arguments)]
async fn drain(
    rtc: &mut Rtc,
    sock: &UdpSocket,
    t0: Instant,
    counts: &mut Counts,
    mid: &mut Option<Mid>,
    connected_at: &mut Option<Instant>,
    ice_log: &mut Vec<String>,
    dead: &mut bool,
) -> Instant {
    loop {
        match rtc.poll_output() {
            Ok(RtcOutput::Timeout(t)) => return t,
            Ok(RtcOutput::Transmit(t)) => {
                *counts
                    .out_by_class
                    .entry(class_detail(&t.contents))
                    .or_default() += 1;
                if let Err(e) = sock.send_to(&t.contents, t.destination).await {
                    step(t0, "udp send error", &e.to_string());
                }
            }
            Ok(RtcOutput::Event(e)) => match e {
                RtcEvent::Connected => {
                    *connected_at = Some(Instant::now());
                    step(t0, "ice+dtls", "Connected (ICE and DTLS-SRTP complete)");
                }
                RtcEvent::IceConnectionStateChange(s) => {
                    let s = format!("{s:?}");
                    step(t0, "ice state", &s);
                    ice_log.push(s.clone());
                    if s == "Disconnected" {
                        *dead = true;
                    }
                }
                RtcEvent::MediaData(d) => {
                    counts.recv += 1;
                    if let Some(dec) = counts.decoder.as_mut() {
                        let mut out = [0f32; 5760];
                        if let Ok(n) = dec.decode_float(&d.data, &mut out, false) {
                            counts.pcm.extend_from_slice(&out[..n]);
                        }
                    }
                }
                RtcEvent::MediaAdded(m) => {
                    step(t0, "media added", "remote media");
                    mid.get_or_insert(m.mid);
                }
                RtcEvent::MediaEgressStats(s) => counts.egress_packets = s.packets,
                RtcEvent::MediaIngressStats(s) => counts.ingress_packets = s.packets,
                _ => {}
            },
            Err(e) => {
                step(t0, "rtc error", &mask(&e.to_string()));
                *dead = true;
                return Instant::now() + Duration::from_secs(1);
            }
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let t0 = Instant::now();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some(team), Some(channel)) = (args.first(), args.get(1)) else {
        eprintln!("usage: native_spike <team id> <channel id>");
        std::process::exit(2);
    };
    let show_sdp = std::env::var_os("SPIKE_SDP").is_some();
    // SPIKE_MULTIDEVICE=A, B or AB sends multidevice=true for that attendee (default: false, as
    // the web client does).
    let md = std::env::var("SPIKE_MULTIDEVICE").unwrap_or_default();
    let a = ROLE.scope(
        "A",
        run(t0, team, channel, show_sdp, Role::Tone, md.contains('A')),
    );
    let b = ROLE.scope(
        "B",
        run(t0, team, channel, show_sdp, Role::Listen, md.contains('B')),
    );
    let (va, vb) = tokio::join!(a, b);
    println!("RESULT A (tone sender): {va}");
    println!("RESULT B (listener): {vb}");
}

/// Goertzel power of `x` at `freq` Hz (48 kHz samples).
fn goertzel(x: &[f32], freq: f64) -> f64 {
    let w = 2.0 * std::f64::consts::PI * freq / 48_000.0;
    let c = 2.0 * w.cos();
    let (mut s1, mut s2) = (0.0f64, 0.0f64);
    for &v in x {
        let s0 = f64::from(v) + c * s1 - s2;
        s2 = s1;
        s1 = s0;
    }
    (s1 * s1 + s2 * s2 - c * s1 * s2) / (x.len() as f64).powi(2)
}

fn rms(x: &[f32]) -> f64 {
    if x.is_empty() {
        return 0.0;
    }
    (x.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>() / x.len() as f64).sqrt()
}

/// Prints RMS overall and of the loudest 1 s window, and the dominant frequency (5 Hz DFT grid,
/// 100 to 2000 Hz) of that window. Returns true when the dominant frequency is within 10 Hz of
/// 440 Hz and the window RMS is above 0.01.
fn analyse(pcm: &[f32]) -> bool {
    let win = 48_000;
    println!(
        "DECODED: samples {} ({:.1} s); overall rms {:.4}",
        pcm.len(),
        pcm.len() as f64 / 48_000.0,
        rms(pcm)
    );
    if pcm.len() < win {
        return false;
    }
    let mut best = (0usize, 0.0f64);
    let mut off = 0;
    while off + win <= pcm.len() {
        let r = rms(&pcm[off..off + win]);
        if r > best.1 {
            best = (off, r);
        }
        off += 4800;
    }
    let w = &pcm[best.0..best.0 + win];
    let (mut top, mut top_p, mut p440) = (0.0, 0.0f64, 0.0f64);
    let mut f = 100.0;
    let mut sum = 0.0;
    let mut n = 0.0;
    while f <= 2000.0 {
        let p = goertzel(w, f);
        sum += p;
        n += 1.0;
        if p > top_p {
            top_p = p;
            top = f;
        }
        if (f - 440.0f64).abs() < 0.1 {
            p440 = p;
        }
        f += 5.0;
    }
    println!(
        "MEASURED: loudest 1 s window starts at {:.1} s, rms {:.4}; dominant frequency {top:.0} Hz; power at 440 Hz is {:.1}x the grid mean",
        best.0 as f64 / 48_000.0,
        best.1,
        p440 / (sum / n)
    );
    (top - 440.0f64).abs() <= 10.0 && best.1 > 0.01
}

async fn run(
    t0: Instant,
    team: &str,
    channel: &str,
    show_sdp: bool,
    role: Role,
    multidevice: bool,
) -> String {
    // 1. Credentials and rooms.join.
    let path = settings::paths::data_dir().join("development_credentials.json");
    let raw = match std::fs::read(&path) {
        Ok(b) => b,
        Err(_) => return "FAIL credentials file unreadable".into(),
    };
    let ws = WorkspaceKey::new("slack", team);
    let json: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(_) => return "FAIL credentials file is not json".into(),
    };
    let Some(entry) = json.get(format!("parla://workspace/{team}")) else {
        return "FAIL no credentials entry for the team".into();
    };
    let user = entry["user"].as_str().unwrap_or("").to_string();
    let value = entry["value"].as_str().unwrap_or("");
    let creds = match slack_auth::decode_credentials(&ws, &user, value.as_bytes()) {
        Ok(c) => c,
        Err(_) => return "FAIL credentials did not decode".into(),
    };
    step(
        t0,
        "credentials",
        "decoded (token and cookie present, not printed)",
    );
    let cfg = ClientConfig::for_workspace(&creds.url);
    let client = match WebClient::new(cfg, creds.token, creds.cookie) {
        Ok(c) => c,
        Err(e) => return format!("FAIL client: {}", mask(&e.to_string())),
    };
    let join = match client.rooms_join_with(channel, multidevice).await {
        Ok(j) => j,
        Err(e) => return format!("FAIL rooms.join: {}", mask(&e.to_string())),
    };
    step(
        t0,
        "rooms.join",
        &format!(
            "ok; signal host {}; audio host {}; token len {}; turn url {}",
            host_of(&join.signaling_url),
            host_of(&join.audio_host_url),
            join.join_token.expose().len(),
            join.turn_control_url.is_some()
        ),
    );

    // 2. Signalling websocket.
    let url = format!(
        "{}?X-Chime-Control-Protocol-Version=3&X-Amzn-Chime-Send-Close-On-Error=1",
        join.signaling_url
    );
    let mut req = match url.into_client_request() {
        Ok(r) => r,
        Err(_) => return "FAIL signalling url is not a websocket request".into(),
    };
    let proto = format!("_aws_wt_session, {}", join.join_token.expose());
    req.headers_mut().insert(
        "Sec-WebSocket-Protocol",
        HeaderValue::from_str(&proto).expect("token is header safe"),
    );
    let (ws_stream, resp) = match tokio_tungstenite::connect_async(req).await {
        Ok(r) => r,
        Err(e) => return format!("FAIL websocket upgrade: {}", mask(&e.to_string())),
    };
    step(
        t0,
        "signalling",
        &format!(
            "upgraded; status {}; server chose a subprotocol {}",
            resp.status().as_u16(),
            resp.headers().contains_key("sec-websocket-protocol")
        ),
    );
    let (mut ws_tx, mut ws_rx) = ws_stream.split();

    // 3. Media: UDP socket, str0m offer.
    let audio_addr: SocketAddr = match tokio::net::lookup_host(&*join.audio_host_url)
        .await
        .ok()
        .and_then(|mut a| a.find(|a| a.is_ipv4()))
    {
        Some(a) => a,
        None => return "FAIL audio host did not resolve".into(),
    };
    // The local IP the OS would route to the audio host (no packet is sent by connect).
    let probe = UdpSocket::bind("0.0.0.0:0").await.expect("bind");
    probe.connect(audio_addr).await.expect("connect");
    let local_ip = probe.local_addr().expect("addr").ip();
    drop(probe);
    let sock = UdpSocket::bind((local_ip, 0)).await.expect("bind udp");
    let local = sock.local_addr().expect("addr");

    let mut rtc = Rtc::builder()
        .clear_codecs()
        .enable_opus(true, false)
        .set_stats_interval(Some(Duration::from_secs(1)))
        .build(Instant::now());
    rtc.add_local_candidate(Candidate::host(local, "udp").expect("candidate"));
    let mut api = rtc.sdp_api();
    let mid = api.add_media(MediaKind::Audio, Direction::SendRecv, None, None, None);
    let (offer, pending) = api.apply().expect("offer has changes");
    let offer_sdp = offer.to_sdp_string();
    step(
        t0,
        "offer",
        &format!(
            "created; {} bytes; local udp port {}",
            offer_sdp.len(),
            local.port()
        ),
    );
    if show_sdp {
        println!("--- offer shape ---\n{}\n---", sdp_shape(&offer_sdp));
    }
    let mut pending: Option<SdpPendingOffer> = Some(pending);

    let mut machine = Machine::new(
        Config {
            app_name: "parla".into(),
            app_version: "spike".into(),
            platform_name: "native".into(),
            os_name: std::env::consts::OS.into(),
            os_version: "0".into(),
            audio_host: join.audio_host_url.clone(),
            audio_session_id: None,
            attendee_id: join.attendee_id.to_string(),
        },
        Instant::now(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    );

    // One 20 ms Opus silence frame (CELT, fullband, mono, code 0), what the listener sends.
    const OPUS_SILENCE: [u8; 3] = [0xF8, 0xFF, 0xFE];
    // The tone sender encodes a 440 Hz sine, 48 kHz mono, 20 ms frames.
    let mut encoder = (role == Role::Tone).then(|| {
        opus::Encoder::new(48_000, opus::Channels::Mono, opus::Application::Audio)
            .expect("opus encoder")
    });
    let mut tone_n: usize = 0;
    // The listener runs longer than the sender so it covers the whole tone.
    let run_for = if role == Role::Listen {
        SEND_FOR + Duration::from_secs(6)
    } else {
        SEND_FOR
    };

    let mut counts = Counts::default();
    if role == Role::Listen {
        counts.decoder =
            Some(opus::Decoder::new(48_000, opus::Channels::Mono).expect("opus decoder"));
    }
    let mut media_mid: Option<Mid> = Some(mid);
    let mut connected_at: Option<Instant> = None;
    let mut ice_log: Vec<String> = Vec::new();
    let mut dead = false;
    let mut answer_at: Option<Instant> = None;
    let mut got_answer = false;
    let mut participants_seen = 0usize;
    let mut media_time: u64 = 0;
    let mut next_frame: Option<Instant> = None;
    let mut leave_sent = false;
    let mut left = false;
    let mut failure: Option<String> = None;
    let mut udp_in = 0u64;
    let mut buf = vec![0u8; 2000];

    // Kick the machine: socket is open.
    let mut pending_outputs = machine.handle(Instant::now(), Input::Opened);
    pending_outputs.extend(machine.handle(Instant::now(), Input::LocalOffer { sdp: offer_sdp }));
    let mut rtc_deadline = drain(
        &mut rtc,
        &sock,
        t0,
        &mut counts,
        &mut media_mid,
        &mut connected_at,
        &mut ice_log,
        &mut dead,
    )
    .await;

    loop {
        // Handle machine outputs.
        for o in std::mem::take(&mut pending_outputs) {
            match o {
                Output::Send(b) => {
                    if ws_tx.send(Message::Binary(b.into())).await.is_err() {
                        failure.get_or_insert("websocket send failed".into());
                    }
                }
                Output::Event(ev) => match ev {
                    Event::TurnCredentials(t) => step(
                        t0,
                        "joinack",
                        &format!(
                            "turn uris {} ttl {}s (secrets not printed)",
                            t.uris.len(),
                            t.ttl_seconds
                        ),
                    ),
                    Event::Index { num_participants } => {
                        step(t0, "index", &format!("participants {num_participants}"))
                    }
                    Event::Speaking(streams) => {
                        step(t0, "speaking", &format!("{} streams", streams.len()))
                    }
                    Event::RemoteAnswer { sdp } => {
                        step(
                            t0,
                            "suback",
                            &format!("SDP answer received; {} bytes", sdp.len()),
                        );
                        if show_sdp {
                            println!("--- answer shape ---\n{}\n---", sdp_shape(&sdp));
                        }
                        got_answer = true;
                        match SdpAnswer::from_sdp_string(&sdp) {
                            Ok(ans) => {
                                let p = pending.take().expect("one offer");
                                match rtc.sdp_api().accept_answer(p, ans) {
                                    Ok(()) => {
                                        answer_at = Some(Instant::now());
                                        step(t0, "answer", "applied by str0m");
                                    }
                                    Err(e) => {
                                        failure.get_or_insert(format!(
                                            "str0m rejected the answer: {}",
                                            mask(&e.to_string())
                                        ));
                                    }
                                }
                            }
                            Err(e) => {
                                failure.get_or_insert(format!(
                                    "answer did not parse: {}",
                                    mask(&e.to_string())
                                ));
                            }
                        }
                        rtc_deadline = drain(
                            &mut rtc,
                            &sock,
                            t0,
                            &mut counts,
                            &mut media_mid,
                            &mut connected_at,
                            &mut ice_log,
                            &mut dead,
                        )
                        .await;
                    }
                    Event::Participants(p) => participants_seen = participants_seen.max(p.len()),
                    Event::Error {
                        status,
                        description,
                    } => {
                        step(
                            t0,
                            "server error",
                            &format!("status {status}: {}", mask(&description)),
                        );
                        failure.get_or_insert(format!(
                            "server error status {status}: {}",
                            mask(&description)
                        ));
                    }
                    Event::Left => {
                        step(t0, "leave", "left (ack or close)");
                        left = true;
                    }
                    Event::Failed(r) => {
                        failure.get_or_insert(format!("machine failed: {}", mask(&r)));
                    }
                },
            }
        }
        if left {
            break;
        }
        if (failure.is_some() || dead) && !leave_sent {
            // Leave cleanly even on failure: flush the leave frame, then stop.
            for o in machine.handle(Instant::now(), Input::Leave) {
                if let Output::Send(b) = o {
                    let _ = ws_tx.send(Message::Binary(b.into())).await;
                }
            }
            if failure.is_none() {
                failure = Some("ICE disconnected".into());
            }
            break;
        }

        // Start and stop the media clock.
        if let Some(c) = connected_at {
            if next_frame.is_none() {
                next_frame = Some(Instant::now());
            }
            if !leave_sent && c.elapsed() >= run_for {
                leave_sent = true;
                step(
                    t0,
                    "send",
                    &format!("20 s done; packets written {}", counts.sent),
                );
                pending_outputs.extend(machine.handle(Instant::now(), Input::Leave));
                continue;
            }
        }
        if let (Some(a), None) = (answer_at, connected_at)
            && a.elapsed() > CONNECT_TIMEOUT
        {
            failure.get_or_insert(format!(
                "media not connected {}s after the answer (ice states: {:?}, udp datagrams in: {udp_in})",
                CONNECT_TIMEOUT.as_secs(),
                ice_log
            ));
            continue;
        }
        if !got_answer && t0.elapsed() > Duration::from_secs(40) {
            failure.get_or_insert("no subscribe answer in 40 s".into());
            continue;
        }

        // Wait for the next thing.
        let mut wake = rtc_deadline;
        if let Some(d) = machine.next_deadline() {
            wake = wake.min(d);
        }
        if let Some(n) = next_frame
            && !leave_sent
        {
            wake = wake.min(n);
        }
        if answer_at.is_some() && connected_at.is_none() {
            wake = wake.min(Instant::now() + Duration::from_millis(500));
        }
        let sleep = tokio::time::sleep_until(tokio::time::Instant::from_std(wake));
        tokio::pin!(sleep);
        tokio::select! {
            m = ws_rx.next() => {
                match m {
                    Some(Ok(Message::Binary(b))) => {
                        pending_outputs.extend(machine.handle(Instant::now(), Input::Frame(b.to_vec())));
                    }
                    Some(Ok(Message::Close(c))) => {
                        step(t0, "ws close", &format!("server closed, code {:?}", c.map(|c| u16::from(c.code))));
                        pending_outputs.extend(machine.handle(Instant::now(), Input::Closed));
                    }
                    Some(Ok(_)) => {}
                    Some(Err(e)) => {
                        step(t0, "ws error", &mask(&e.to_string()));
                        pending_outputs.extend(machine.handle(Instant::now(), Input::Closed));
                    }
                    None => pending_outputs.extend(machine.handle(Instant::now(), Input::Closed)),
                }
            }
            r = sock.recv_from(&mut buf) => {
                if let Ok((n, src)) = r {
                    udp_in += 1;
                    *counts.in_by_class.entry(class_detail(&buf[..n])).or_default() += 1;
                    if n == 15 && buf[0] == 21 {
                        // A DTLS alert record: level and description are not secret.
                        step(t0, "dtls alert", &format!("level {} description {}", buf[13], buf[14]));
                    }
                    let now = Instant::now();
                    if let Ok(contents) = buf[..n].try_into() {
                        let _ = rtc.handle_input(RtcInput::Receive(now, Receive {
                            proto: Protocol::Udp,
                            source: src,
                            destination: local,
                            contents,
                        }));
                    }
                    rtc_deadline = drain(&mut rtc, &sock, t0, &mut counts, &mut media_mid, &mut connected_at, &mut ice_log, &mut dead).await;
                }
            }
            _ = &mut sleep => {
                let now = Instant::now();
                if now >= machine.next_deadline().unwrap_or(now + Duration::from_secs(3600)) {
                    pending_outputs.extend(machine.handle(now, Input::Tick));
                }
                if let Some(n) = next_frame
                    && now >= n
                    && !leave_sent
                    && connected_at.is_some()
                {
                    let pt = rtc
                        .writer(mid)
                        .and_then(|w| w.payload_params().next().map(|p| p.pt()));
                    if let (Some(pt), Some(w)) = (pt, rtc.writer(mid)) {
                        let mt = MediaTime::new(media_time, Frequency::FORTY_EIGHT_KHZ);
                        let payload = match encoder.as_mut() {
                            Some(enc) => {
                                let pcm: Vec<f32> = (0..960)
                                    .map(|i| {
                                        let t = (tone_n + i) as f64 / 48_000.0;
                                        (0.3 * (2.0 * std::f64::consts::PI * 440.0 * t).sin()) as f32
                                    })
                                    .collect();
                                tone_n += 960;
                                let mut out = [0u8; 400];
                                let n = enc.encode_float(&pcm, &mut out).expect("encode");
                                out[..n].to_vec()
                            }
                            None => OPUS_SILENCE.to_vec(),
                        };
                        if w.write(pt, now, mt, payload).is_ok() {
                            counts.sent += 1;
                        }
                    }
                    // Drain after the write, before the next mutation.
                    drain(&mut rtc, &sock, t0, &mut counts, &mut media_mid, &mut connected_at, &mut ice_log, &mut dead).await;
                    media_time += 960;
                    next_frame = Some(n + FRAME);
                }
                let _ = rtc.handle_input(RtcInput::Timeout(now));
                rtc_deadline = drain(&mut rtc, &sock, t0, &mut counts, &mut media_mid, &mut connected_at, &mut ice_log, &mut dead).await;
            }
        }
    }

    let _ = ws_tx.close().await;
    step(t0, "closed", "websocket closed");
    let connected = connected_at.is_some();
    println!(
        "UDP: out {:?} ; in {:?}",
        counts.out_by_class, counts.in_by_class
    );
    println!(
        "COUNTS: media datagrams written {} ; media packets received {} ; stats egress packets {} ; stats ingress packets {} ; participants max seen {} ; udp datagrams in {}",
        counts.sent,
        counts.recv,
        counts.egress_packets,
        counts.ingress_packets,
        participants_seen,
        udp_in
    );
    let heard = if role == Role::Listen {
        analyse(&counts.pcm)
    } else {
        true
    };
    match (
        failure,
        connected,
        counts.sent > 0,
        (counts.recv > 0 || counts.ingress_packets > 0) && heard,
    ) {
        (None, true, true, true) => "PASS".into(),
        (None, true, true, false) => {
            "PARTIAL connected and sent, the expected audio was not received".into()
        }
        (Some(f), c, _, _) => format!(
            "{} (connected {c}): {f}",
            if c { "PARTIAL" } else { "FAIL" }
        ),
        (None, c, _, _) => format!("FAIL connected {c}"),
    }
}

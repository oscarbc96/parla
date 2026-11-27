mod support {
    pub mod fake_server;
}

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use redact::record::Recorder;
use slack_api::{ApiError, SessionCookie};
use slack_realtime::conn::{ConnConfig, ConnEvent, UrlSource, spawn};
use slack_realtime::event::Outgoing;
use slack_redact::Rules;
use support::fake_server::{FakeServer, Script, Step};
use url::Url;

struct Urls(Url);

impl UrlSource for Urls {
    fn fetch(&self) -> BoxFuture<'_, Result<Url, ApiError>> {
        let u = self.0.clone();
        Box::pin(async move { Ok(u) })
    }
}

#[tokio::test]
async fn record_writes_ws_frames() {
    let server = FakeServer::start(vec![Script {
        steps: vec![
            Step::Send(r#"{"type":"hello"}"#.into()),
            Step::Send(
                r#"{"type":"message","channel":"C0123ABCD45","ts":"1.2","text":"mail me at real.person@corp.com xoxc-1-2-3-abcdef"}"#
                    .into(),
            ),
            Step::Sleep(Duration::from_secs(2)),
        ],
        ..Script::default()
    }])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let rec = Recorder::new(dir.path(), Arc::new(Rules::default())).unwrap();
    let mut cfg = ConnConfig::new(SessionCookie::parse("xoxd-synthetic%2Fcookie").unwrap());
    cfg.recorder = Some(Arc::new(rec));
    let (h, mut rx) = spawn(
        cfg,
        Arc::new(Urls(server.url())),
        &tokio::runtime::Handle::current(),
    );
    let mut seen = 0;
    // Connected, Hello and Message: every inbound frame is recorded before its event is
    // emitted, so after this the two receive files exist and the ping is numbered 3.
    while seen < 3 {
        if let Some(ConnEvent::Event(_)) | Some(ConnEvent::Connected { .. }) = rx.recv().await {
            seen += 1;
        }
    }
    h.send(Outgoing::Ping { id: 7 });
    // The ping is recorded before it is written, so receipt means it is on disk.
    server.wait_received(1).await;
    h.shutdown();

    let names: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        names.iter().any(|n| n.starts_with("ws-0001-hello")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.starts_with("ws-0002-message")),
        "{names:?}"
    );
    assert!(
        names.iter().any(|n| n.starts_with("ws-0003-ping")),
        "{names:?}"
    );
    for n in &names {
        let text = std::fs::read_to_string(dir.path().join(n)).unwrap();
        assert!(
            !text.contains("xox") && !text.contains("corp.com") && !text.contains("C0123ABCD45"),
            "{n}: {text}"
        );
    }
}

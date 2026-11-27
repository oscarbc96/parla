use std::sync::Arc;
use std::time::Instant;

use domain::test_support::{cid, msg, ts, uid};
use domain::{
    EmojiRef, ExtCodec, ExtKind, FileId, FileKind, FileRef, Mention, NoExtCodec, PersistOp,
    ProviderExt, Reaction, RemoteUrl,
};
use settings::WorkspaceKey;
use store::Store;
use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};

/// Encodes a `u32` extension as four little-endian bytes.
struct U32Codec;

impl ExtCodec for U32Codec {
    fn encode(&self, _: ExtKind, ext: &ProviderExt) -> Option<Vec<u8>> {
        ext.get::<u32>().map(|v| v.to_le_bytes().to_vec())
    }
    fn decode(&self, _: ExtKind, bytes: &[u8]) -> ProviderExt {
        match <[u8; 4]>::try_from(bytes) {
            Ok(b) => ProviderExt::new(u32::from_le_bytes(b)),
            Err(_) => ProviderExt::none(),
        }
    }
}

fn w() -> WorkspaceKey {
    WorkspaceKey::new("fake", "W1")
}

fn start(dir: &std::path::Path) -> SqliteStore {
    SqliteStore::start(
        SqliteConfig {
            dir: dir.to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    )
}

async fn put(s: &SqliteStore, c: &str, messages: Vec<domain::Message>) {
    s.apply(
        &w(),
        PersistOp::Messages {
            conversation: cid(c),
            messages,
        },
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn ext_round_trips_through_codec() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path());
    s.open(&w(), Arc::new(U32Codec)).await.unwrap();
    let mut m = msg("C1", "1.000000", "hi");
    m.ext = ProviderExt::new(42u32);
    put(&s, "C1", vec![m]).await;
    s.close(&w()).await.unwrap();
    s.open(&w(), Arc::new(U32Codec)).await.unwrap();
    let got = s.messages_before(&w(), &cid("C1"), None, 10).await.unwrap();
    assert_eq!(got[0].ext.get::<u32>(), Some(&42));
}

#[tokio::test]
async fn files_and_reactions_survive_reopen() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path());
    s.open(&w(), Arc::new(NoExtCodec)).await.unwrap();
    let mut m = msg("C1", "1.000000", "hi");
    m.mentions = Arc::from([Mention::User(uid("u1")), Mention::Group(Arc::from("g"))]);
    m.reply_users = Arc::from([uid("u1")]);
    m.files = Arc::from([FileRef {
        id: FileId(Arc::from("F1")),
        name: Arc::from("a.png"),
        title: Some(Arc::from("A")),
        mime: Arc::from("image/png"),
        size: 9,
        kind: FileKind::Image,
        thumbs: Arc::from([]),
        download: Some(RemoteUrl(Arc::from("https://h/f"))),
        preview: None,
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }]);
    m.reactions = Arc::from([Reaction {
        emoji: EmojiRef {
            shortcode: Arc::from("+1"),
            unicode: Some(Arc::from("x")),
            skin_tone: Some(3),
        },
        count: 2,
        users: Arc::from([uid("u1")]),
        me: true,
    }]);
    put(&s, "C1", vec![m.clone()]).await;
    s.close(&w()).await.unwrap();
    s.open(&w(), Arc::new(NoExtCodec)).await.unwrap();
    let got = s.messages_before(&w(), &cid("C1"), None, 10).await.unwrap();
    assert_eq!(got[0].mentions, m.mentions);
    assert_eq!(got[0].reply_users, m.reply_users);
    assert_eq!(got[0].reactions, m.reactions);
    assert_eq!(got[0].files.len(), 1);
    assert_eq!(got[0].files[0].id, m.files[0].id);
    assert_eq!(got[0].files[0].download, m.files[0].download);
    assert_eq!(got[0].files[0].title, m.files[0].title);

    s.apply(&w(), PersistOp::FileDeleted(FileId(Arc::from("F1"))))
        .await
        .unwrap();
    let got = s.messages_before(&w(), &cid("C1"), None, 10).await.unwrap();
    assert!(got[0].files[0].tombstone);
}

#[tokio::test]
async fn ordering_is_numeric_not_lexical() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path());
    s.open(&w(), Arc::new(NoExtCodec)).await.unwrap();
    put(
        &s,
        "C1",
        vec![msg("C1", "10.000001", "b"), msg("C1", "9.000002", "a")],
    )
    .await;
    let got = s.messages_before(&w(), &cid("C1"), None, 10).await.unwrap();
    assert_eq!(got[0].id.ts, ts("10.000001"));
    assert_eq!(got[1].id.ts, ts("9.000002"));
    let older = s
        .messages_before(&w(), &cid("C1"), Some(ts("10.000001")), 10)
        .await
        .unwrap();
    assert_eq!(older.len(), 1);
    assert_eq!(
        s.latest_ts(&w(), &cid("C1")).await.unwrap(),
        Some(ts("10.000001"))
    );
}

#[tokio::test]
async fn upsert_keeps_client_msg_id_and_ts() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path());
    s.open(&w(), Arc::new(NoExtCodec)).await.unwrap();
    let mut a = msg("C1", "1.000000", "pending");
    a.client_msg_id = Some(domain::test_support::cmid("cm"));
    put(&s, "C1", vec![a]).await;
    put(&s, "C1", vec![msg("C1", "1.000000", "echo")]).await;
    let got = s.messages_before(&w(), &cid("C1"), None, 10).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(&*got[0].text, "echo");
    assert_eq!(got[0].client_msg_id, Some(domain::test_support::cmid("cm")));
}

#[tokio::test]
async fn batch_of_1000_messages_is_one_transaction() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path());
    s.open(&w(), Arc::new(NoExtCodec)).await.unwrap();
    let batch: Vec<_> = (1..=1000)
        .map(|i| msg("C1", &format!("{i}.000000"), "x"))
        .collect();
    let t = Instant::now();
    put(&s, "C1", batch).await;
    assert!(t.elapsed().as_secs_f64() < 2.0, "took {:?}", t.elapsed());
    let got = s
        .messages_before(&w(), &cid("C1"), None, 2000)
        .await
        .unwrap();
    assert_eq!(got.len(), 1000);
}

/// A blob written before a field existed still decodes (`#[serde(default)]`).
#[tokio::test]
async fn old_file_blob_missing_fields_still_decodes() {
    let d = tempfile::tempdir().unwrap();
    let s = start(d.path());
    s.open(&w(), Arc::new(NoExtCodec)).await.unwrap();
    put(&s, "C1", vec![msg("C1", "1000.000001", "hi")]).await;
    s.write(&w(), |c, _| {
        c.execute(
            "UPDATE messages SET files = ?1",
            [
                br#"[{"id":"F1","name":"a.txt","mime":"text/plain","size":3,"kind":"Other"}]"#
                    .to_vec(),
            ],
        )
        .map(|_| ())
        .map_err(|e| store::StoreError::Other(e.to_string()))
    })
    .await
    .unwrap();
    let got = s.messages_before(&w(), &cid("C1"), None, 10).await.unwrap();
    assert_eq!(got[0].files.len(), 1);
    assert!(!got[0].files[0].tombstone);
}

//! A member's handle and a group's participants survive a reopen, so the
//! sidebar can name groups from the cache before the first bootstrap.

use std::sync::Arc;

use domain::test_support::{conv, member};
use domain::{ConversationKind, NoExtCodec, PersistOp};
use settings::WorkspaceKey;
use store::Store;
use store_sqlite::{MemoryKeySource, SqliteConfig, SqliteStore};

#[tokio::test]
async fn handle_and_participants_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let s = SqliteStore::start(
        SqliteConfig {
            dir: dir.path().to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    );
    let w = WorkspaceKey::new("fake", "w1");
    s.open(&w, Arc::new(NoExtCodec)).await.unwrap();
    let mut g = conv("G1", "mpdm-a--b-1");
    g.kind = ConversationKind::Group;
    g.participants = ["amy", "bob"]
        .iter()
        .map(|h| Arc::<str>::from(*h))
        .collect();
    let mut m = member("u1", "Amy A");
    m.handle = Arc::from("amy");
    s.apply(&w, PersistOp::Conversations(vec![g, conv("C1", "general")]))
        .await
        .unwrap();
    s.apply(&w, PersistOp::Members(vec![m])).await.unwrap();
    let cs = s.conversations(&w).await.unwrap();
    let g = cs.iter().find(|c| &*c.id.0 == "G1").unwrap();
    let handles: Vec<&str> = g.participants.iter().map(|h| &**h).collect();
    assert_eq!(handles, ["amy", "bob"]);
    assert!(
        cs.iter()
            .find(|c| &*c.id.0 == "C1")
            .unwrap()
            .participants
            .is_empty()
    );
    assert_eq!(&*s.members(&w).await.unwrap()[0].handle, "amy");
}

/// The directory's title and time zone survive a reopen, so the People view
/// shows them from the cache before the first bootstrap.
#[tokio::test]
async fn title_and_time_zone_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let s = SqliteStore::start(
        SqliteConfig {
            dir: dir.path().to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    );
    let w = WorkspaceKey::new("fake", "w1");
    s.open(&w, Arc::new(NoExtCodec)).await.unwrap();
    let mut m = member("u1", "Amy A");
    m.title = Some(Arc::from("Design lead"));
    m.tz = Some(Arc::from("Europe/Madrid"));
    s.apply(&w, PersistOp::Members(vec![m, member("u2", "Bob B")]))
        .await
        .unwrap();
    let ms = s.members(&w).await.unwrap();
    assert_eq!(ms[0].title.as_deref(), Some("Design lead"));
    assert_eq!(ms[0].tz.as_deref(), Some("Europe/Madrid"));
    assert_eq!(ms[1].title, None);
    assert_eq!(ms[1].tz, None);
}

/// A member's picture survives a reopen, so rows show it before the first bootstrap.
#[tokio::test]
async fn avatar_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let s = SqliteStore::start(
        SqliteConfig {
            dir: dir.path().to_path_buf(),
            cache_mb: 8,
        },
        Arc::new(MemoryKeySource::default()),
    );
    let w = WorkspaceKey::new("fake", "w1");
    s.open(&w, Arc::new(NoExtCodec)).await.unwrap();
    let mut a = member("u1", "Amy A");
    a.avatar = domain::Avatar::new(
        Some(domain::ImageRef::Provider(domain::RemoteUrl(
            "https://x.test/a.png".into(),
        ))),
        None,
    );
    let mut b = member("u2", "Bob B");
    b.avatar = domain::Avatar::new(
        Some(domain::ImageRef::Web("https://y.test/b.png".into())),
        None,
    );
    s.apply(
        &w,
        PersistOp::Members(vec![a.clone(), b.clone(), member("u3", "Cy C")]),
    )
    .await
    .unwrap();
    let ms = s.members(&w).await.unwrap();
    assert_eq!(ms[0].avatar, a.avatar);
    assert_eq!(ms[1].avatar, b.avatar);
    assert_eq!(ms[2].avatar, None);
    // The large one is kept apart from the row one.
    let (row, lg) = (
        domain::ImageRef::Web("https://y.test/row.png".into()),
        domain::ImageRef::Web("https://y.test/lg.png".into()),
    );
    let mut c = member("u4", "Di D");
    c.avatar = domain::Avatar::new(Some(row.clone()), Some(lg.clone()));
    s.apply(&w, PersistOp::Members(vec![c])).await.unwrap();
    let ms = s.members(&w).await.unwrap();
    let got = ms.iter().find(|m| &*m.id.0 == "u4").unwrap();
    assert_eq!(got.avatar_for(36.0), Some(&row));
    assert_eq!(got.avatar_for(72.0), Some(&lg));
}

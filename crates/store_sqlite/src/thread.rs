//! The `parla-store` writer thread and per-database reader threads.

use std::any::Any;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::{Builder, JoinHandle};

use domain::ExtCodec;
use futures::channel::oneshot;
use futures::executor::block_on;
use futures::future::BoxFuture;
use rusqlite::{Connection, OpenFlags};
use settings::WorkspaceKey;
use store::StoreError;

use crate::conn;
use crate::free_space::{FREE_SPACE_CHECK_MS, FreeSpace, SystemFreeSpace};
use crate::key::{DbId, KeySource};
use crate::migrate;

/// Commands drained into one transaction per pass.
const BATCH: usize = 256;
const READERS: usize = 2;

#[derive(Debug, Clone)]
pub struct SqliteConfig {
    /// `settings::paths::data_dir().join("store")`.
    pub dir: PathBuf,
    /// Page cache budget in MiB, split across open databases.
    pub cache_mb: u32,
}

type Reply<T> = oneshot::Sender<Result<T, StoreError>>;
type Boxed = Box<dyn Any + Send>;
type WriteJob =
    Box<dyn FnOnce(&Connection, &Arc<dyn ExtCodec>) -> Result<Boxed, StoreError> + Send>;
type ReadJob = Box<dyn FnOnce(&Connection, &Arc<dyn ExtCodec>) + Send>;

enum Cmd {
    Open(DbId, Arc<dyn ExtCodec>, Reply<()>),
    Close(DbId, Reply<()>),
    Rebuild(DbId, Reply<()>),
    Remove(DbId, Reply<()>),
    Write(DbId, WriteJob, Reply<Boxed>),
}

struct Shared {
    cfg: SqliteConfig,
    cmd: Mutex<Option<Sender<Cmd>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    readers: Mutex<HashMap<DbId, Sender<ReadJob>>>,
    /// Unix ms of the last free-space read; 0 before the first.
    last_space_check: AtomicI64,
    free_space: Mutex<Arc<dyn FreeSpace>>,
    #[cfg(feature = "test-support")]
    last_writer: Mutex<Option<String>>,
}

impl Shared {
    /// Stops accepting commands, lets the readers end, then waits for the
    /// writer thread to close every database.
    fn stop(&self) {
        self.cmd.lock().unwrap().take();
        self.readers.lock().unwrap().clear(); // lets reader threads end
        if let Some(h) = self.thread.lock().unwrap().take()
            && h.thread().id() != std::thread::current().id()
        {
            let _ = h.join();
        }
    }
}

impl Drop for Shared {
    /// Wait for the databases to close, so none is mid-teardown at process exit.
    fn drop(&mut self) {
        self.stop();
    }
}

/// Cheap to clone; all clones share one `parla-store` thread. The thread ends
/// (and closes every database) when the last clone is dropped.
#[derive(Clone)]
pub struct SqliteStore(Arc<Shared>);

impl SqliteStore {
    /// Replaces where free space is read from (tests inject a fixed value).
    pub fn set_free_space(&self, source: Arc<dyn FreeSpace>) {
        *self.0.free_space.lock().unwrap() = source;
    }

    /// Takes the next free-space check slot: true at most once per
    /// [`FREE_SPACE_CHECK_MS`] over every clone of the store (the first call
    /// always). `now_ms` is Unix milliseconds. Cheap, no IO.
    pub fn claim_free_space_check(&self, now_ms: i64) -> bool {
        let last = self.0.last_space_check.load(Ordering::Relaxed);
        if last != 0 && now_ms.saturating_sub(last) < FREE_SPACE_CHECK_MS {
            return false;
        }
        self.0
            .last_space_check
            .compare_exchange(last, now_ms.max(1), Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }

    /// Free bytes on the volume of the store now. A statvfs call; run it off
    /// the main thread.
    pub fn read_free_space(&self) -> Option<u64> {
        let source = self.0.free_space.lock().unwrap().clone();
        source.free_bytes(&self.0.cfg.dir)
    }

    /// [`claim_free_space_check`](Self::claim_free_space_check) then
    /// [`read_free_space`](Self::read_free_space): `None` while throttled.
    pub fn check_free_space(&self, now_ms: i64) -> Option<u64> {
        self.claim_free_space_check(now_ms)
            .then(|| self.read_free_space())
            .flatten()
    }

    pub fn start(cfg: SqliteConfig, keys: Arc<dyn KeySource>) -> SqliteStore {
        let (tx, rx) = channel();
        let shared = Arc::new(Shared {
            cfg,
            cmd: Mutex::new(Some(tx)),
            thread: Mutex::default(),
            readers: Mutex::default(),
            last_space_check: AtomicI64::new(0),
            free_space: Mutex::new(Arc::new(SystemFreeSpace)),
            #[cfg(feature = "test-support")]
            last_writer: Mutex::default(),
        });
        let worker = Worker {
            shared: Arc::downgrade(&shared),
            cfg: shared.cfg.clone(),
            keys,
            dbs: HashMap::new(),
            codecs: HashMap::new(),
        };
        let handle = Builder::new()
            .name("parla-store".into())
            .spawn(move || worker.run(rx))
            .expect("spawn parla-store thread");
        *shared.thread.lock().unwrap() = Some(handle);
        SqliteStore(shared)
    }

    /// Closes every database and joins the store threads, even while clones
    /// of the handle are alive; later calls fail with `StoreError::Closed`.
    ///
    /// Call it before the process exits. SQLCipher registers an `atexit`
    /// hook that frees its global crypto state, so a store thread still
    /// running then can touch freed memory and crash (SIGSEGV).
    pub fn shutdown(&self) {
        self.0.stop();
    }

    pub fn path(&self, ws: &WorkspaceKey) -> PathBuf {
        db_path(&self.0.cfg.dir, &DbId::Workspace(ws.clone()))
    }

    /// Path of the cross-workspace `app.db`.
    pub fn app_path(&self) -> PathBuf {
        db_path(&self.0.cfg.dir, &DbId::App)
    }

    pub fn open(
        &self,
        ws: &WorkspaceKey,
        codec: Arc<dyn ExtCodec>,
    ) -> BoxFuture<'_, Result<(), StoreError>> {
        self.call(|r| Cmd::Open(DbId::Workspace(ws.clone()), codec, r))
    }

    pub fn close(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<(), StoreError>> {
        self.call(|r| Cmd::Close(DbId::Workspace(ws.clone()), r))
    }

    /// Opens (creating and migrating) the cross-workspace `app.db`, on the
    /// same writer thread and with the same pragmas as workspace databases.
    pub fn open_app(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.call(|r| Cmd::Open(DbId::App, Arc::new(domain::NoExtCodec), r))
    }

    /// Closes `app.db` (test-support only: a test closes it to checkpoint).
    #[cfg(feature = "test-support")]
    pub fn close_app(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.call(|r| Cmd::Close(DbId::App, r))
    }

    /// As [`rebuild`](Self::rebuild), for `app.db`.
    pub fn rebuild_app(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.call(|r| Cmd::Rebuild(DbId::App, r))
    }

    /// Closes, deletes the file and its `-wal`/`-shm`, replaces the key and
    /// reopens empty. Works on a database that failed to open.
    pub fn rebuild(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<(), StoreError>> {
        self.call(|r| Cmd::Rebuild(DbId::Workspace(ws.clone()), r))
    }

    /// Closes the database, deletes the file and its `-wal`/`-shm`, then
    /// deletes the key. Works on a database that is not open.
    pub fn remove(&self, ws: &WorkspaceKey) -> BoxFuture<'_, Result<(), StoreError>> {
        self.call(|r| Cmd::Remove(DbId::Workspace(ws.clone()), r))
    }

    /// As [`remove`](Self::remove), for the cross-workspace `app.db`.
    pub fn remove_app(&self) -> BoxFuture<'_, Result<(), StoreError>> {
        self.call(|r| Cmd::Remove(DbId::App, r))
    }

    /// Runs `f` on the writer thread; jobs queued together share a
    /// transaction, each in its own savepoint (an `Err` rolls back only that job).
    pub fn write<T, F>(&self, ws: &WorkspaceKey, f: F) -> BoxFuture<'_, Result<T, StoreError>>
    where
        T: Send + 'static,
        F: FnOnce(&Connection, &Arc<dyn ExtCodec>) -> Result<T, StoreError> + Send + 'static,
    {
        self.write_db(&DbId::Workspace(ws.clone()), f)
    }

    pub(crate) fn write_db<T, F>(&self, ws: &DbId, f: F) -> BoxFuture<'_, Result<T, StoreError>>
    where
        T: Send + 'static,
        F: FnOnce(&Connection, &Arc<dyn ExtCodec>) -> Result<T, StoreError> + Send + 'static,
    {
        let fut = self.call(|r| {
            Cmd::Write(
                ws.clone(),
                Box::new(move |c, codec| f(c, codec).map(|v| Box::new(v) as Boxed)),
                r,
            )
        });
        Box::pin(async move {
            let b = fut.await?;
            Ok(*b.downcast::<T>().expect("write result type"))
        })
    }

    /// Runs `f` on one of the database's two reader threads.
    pub fn read<T, F>(&self, ws: &WorkspaceKey, f: F) -> BoxFuture<'_, Result<T, StoreError>>
    where
        T: Send + 'static,
        F: FnOnce(&Connection, &Arc<dyn ExtCodec>) -> Result<T, StoreError> + Send + 'static,
    {
        self.read_db(&DbId::Workspace(ws.clone()), f)
    }

    pub(crate) fn read_db<T, F>(&self, ws: &DbId, f: F) -> BoxFuture<'_, Result<T, StoreError>>
    where
        T: Send + 'static,
        F: FnOnce(&Connection, &Arc<dyn ExtCodec>) -> Result<T, StoreError> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        let sent = match self.0.readers.lock().unwrap().get(ws) {
            Some(q) => q
                .send(Box::new(move |c, codec| {
                    let _ = tx.send(f(c, codec));
                }))
                .is_ok(),
            None => false,
        };
        Box::pin(async move {
            if !sent {
                return Err(StoreError::Closed);
            }
            rx.await.map_err(|_| StoreError::Closed)?
        })
    }

    fn call<T: Send + 'static>(
        &self,
        mk: impl FnOnce(Reply<T>) -> Cmd,
    ) -> BoxFuture<'_, Result<T, StoreError>> {
        let (tx, rx) = oneshot::channel();
        let sent = match self.0.cmd.lock().unwrap().as_ref() {
            Some(q) => q.send(mk(tx)).is_ok(),
            None => false,
        };
        Box::pin(async move {
            if !sent {
                return Err(StoreError::Closed);
            }
            rx.await.map_err(|_| StoreError::Closed)?
        })
    }

    /// Name of the thread that last ran a write job.
    #[cfg(feature = "test-support")]
    pub fn last_writer_thread(&self) -> Option<String> {
        self.0.last_writer.lock().unwrap().clone()
    }

    /// `EXPLAIN QUERY PLAN` detail lines of the message-window query.
    #[cfg(feature = "test-support")]
    pub fn explain_window(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<String>, StoreError>> {
        self.explain(ws, crate::store_impl::window_sql())
    }

    /// `EXPLAIN QUERY PLAN` detail lines of the Files tab query.
    #[cfg(feature = "test-support")]
    pub fn explain_files(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<String>, StoreError>> {
        self.explain(ws, crate::store_impl::files_sql())
    }

    /// `EXPLAIN QUERY PLAN` detail lines of the per-conversation latest query.
    #[cfg(feature = "test-support")]
    pub fn explain_latest(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<String>, StoreError>> {
        let sql = format!("EXPLAIN QUERY PLAN {}", crate::store_impl::latest_sql());
        self.read(ws, move |c, _| {
            let other = crate::err::other;
            let mut st = c.prepare(&sql).map_err(other)?;
            let rows = st.query_map([], |r| r.get::<_, String>(3)).map_err(other)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(other)
        })
    }

    #[cfg(feature = "test-support")]
    fn explain(
        &self,
        ws: &WorkspaceKey,
        query: String,
    ) -> BoxFuture<'_, Result<Vec<String>, StoreError>> {
        // The Files query also binds the cursor's text (`?4`).
        let with_ts = query.contains("?4");
        let sql = format!("EXPLAIN QUERY PLAN {query}");
        self.read(ws, move |c, _| {
            let other = crate::err::other;
            let mut st = c.prepare(&sql).map_err(other)?;
            let rows = st
                .query_map(
                    rusqlite::params_from_iter(
                        [
                            rusqlite::types::Value::from("C0000".to_string()),
                            i64::MAX.into(),
                            200_i64.into(),
                            String::new().into(),
                        ]
                        .into_iter()
                        .take(if with_ts { 4 } else { 3 }),
                    ),
                    |r| r.get::<_, String>(3),
                )
                .map_err(other)?;
            rows.collect::<Result<Vec<_>, _>>().map_err(other)
        })
    }

    /// Value of `PRAGMA <name>` on the writer connection.
    #[cfg(feature = "test-support")]
    pub fn pragma(
        &self,
        ws: &WorkspaceKey,
        name: &str,
    ) -> BoxFuture<'_, Result<String, StoreError>> {
        self.pragma_db(DbId::Workspace(ws.clone()), name)
    }

    /// Value of `PRAGMA <name>` on `app.db`'s writer connection.
    #[cfg(feature = "test-support")]
    pub fn app_pragma(&self, name: &str) -> BoxFuture<'_, Result<String, StoreError>> {
        self.pragma_db(DbId::App, name)
    }

    #[cfg(feature = "test-support")]
    fn pragma_db(&self, ws: DbId, name: &str) -> BoxFuture<'_, Result<String, StoreError>> {
        use rusqlite::types::Value;
        let sql = format!("PRAGMA {name}");
        self.write_db(&ws, move |c, _| {
            let v: Value = c
                .query_row(&sql, [], |r| r.get(0))
                .map_err(crate::err::other)?;
            Ok(match v {
                Value::Integer(i) => i.to_string(),
                Value::Text(t) => t,
                other => format!("{other:?}"),
            })
        })
    }
}

fn db_path(dir: &std::path::Path, id: &DbId) -> PathBuf {
    match id {
        DbId::App => dir.join("app.db"),
        DbId::Workspace(ws) => {
            let (provider, id) = ws.path_parts();
            dir.join(provider).join(format!("{id}.db"))
        }
    }
}

struct Db {
    writer: Connection,
    readers: Vec<JoinHandle<()>>,
    cache_kib: Arc<AtomicI64>,
}

struct Worker {
    shared: std::sync::Weak<Shared>,
    cfg: SqliteConfig,
    keys: Arc<dyn KeySource>,
    dbs: HashMap<DbId, Db>,
    codecs: HashMap<DbId, Arc<dyn ExtCodec>>,
}

impl Worker {
    fn run(mut self, rx: Receiver<Cmd>) {
        while let Ok(first) = rx.recv() {
            let mut batch = vec![first];
            while batch.len() < BATCH {
                match rx.try_recv() {
                    Ok(c) => batch.push(c),
                    Err(_) => break,
                }
            }
            self.process(batch);
        }
        let all: Vec<_> = self.dbs.keys().cloned().collect();
        for ws in all {
            self.close(&ws);
        }
    }

    fn process(&mut self, batch: Vec<Cmd>) {
        let mut it = batch.into_iter().peekable();
        while let Some(cmd) = it.next() {
            match cmd {
                Cmd::Open(ws, codec, r) => {
                    let _ = r.send(self.open(&ws, codec));
                }
                Cmd::Close(ws, r) => {
                    self.close(&ws);
                    let _ = r.send(Ok(()));
                }
                Cmd::Rebuild(ws, r) => {
                    let _ = r.send(self.rebuild(&ws));
                }
                Cmd::Remove(ws, r) => {
                    let _ = r.send(self.remove(&ws));
                }
                Cmd::Write(ws, job, reply) => {
                    let mut run = vec![(job, reply)];
                    while let Some(Cmd::Write(next, ..)) = it.peek() {
                        if *next != ws {
                            break;
                        }
                        if let Some(Cmd::Write(_, j, r)) = it.next() {
                            run.push((j, r));
                        }
                    }
                    self.write_run(&ws, run);
                }
            }
        }
    }

    fn write_run(&mut self, ws: &DbId, run: Vec<(WriteJob, Reply<Boxed>)>) {
        #[cfg(feature = "test-support")]
        if let Some(s) = self.shared.upgrade() {
            *s.last_writer.lock().unwrap() = std::thread::current().name().map(str::to_owned);
        }
        let (Some(db), Some(codec)) = (self.dbs.get_mut(ws), self.codecs.get(ws)) else {
            for (_, r) in run {
                let _ = r.send(Err(StoreError::Closed));
            }
            return;
        };
        let other = crate::err::other;
        let mut tx = match db.writer.transaction().map_err(other) {
            Ok(tx) => tx,
            Err(e) => {
                for (_, r) in run {
                    let _ = r.send(Err(e.clone()));
                }
                return;
            }
        };
        let mut results = Vec::with_capacity(run.len());
        for (job, reply) in run {
            let res = match tx.savepoint().map_err(other) {
                Ok(sp) => match job(&sp, codec) {
                    Ok(v) => sp.commit().map_err(other).map(|_| v),
                    Err(e) => Err(e), // dropping the savepoint rolls it back
                },
                Err(e) => Err(e),
            };
            results.push((reply, res));
        }
        match tx.commit().map_err(other) {
            Ok(()) => {
                for (r, res) in results {
                    let _ = r.send(res);
                }
            }
            Err(e) => {
                for (r, _) in results {
                    let _ = r.send(Err(e.clone()));
                }
            }
        }
    }

    fn open(&mut self, ws: &DbId, codec: Arc<dyn ExtCodec>) -> Result<(), StoreError> {
        self.codecs.insert(ws.clone(), codec.clone());
        if self.dbs.contains_key(ws) {
            return Ok(());
        }
        let io = crate::err::other;
        let path = db_path(&self.cfg.dir, ws);
        let parent = path.parent().expect("db path has a parent");
        conn::create_private_dir(&self.cfg.dir).map_err(io)?;
        conn::create_private_dir(parent).map_err(io)?;
        conn::create_private_file(&path).map_err(io)?;
        let key = block_on(self.keys.get_or_create(ws))?;

        let rw = OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let mut writer = conn::open_keyed(&path, &key, rw)?;
        conn::apply_writer_pragmas(&writer)?;
        match ws {
            DbId::App => migrate::migrate_app(&mut writer)?,
            DbId::Workspace(_) => migrate::migrate_workspace(&mut writer)?,
        }

        let ro = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let (jobs_tx, jobs_rx) = channel::<ReadJob>();
        let jobs_rx = Arc::new(Mutex::new(jobs_rx));
        let cache_kib = Arc::new(AtomicI64::new(0));
        let mut readers = Vec::new();
        for _ in 0..READERS {
            let c = conn::open_keyed(&path, &key, ro)?;
            conn::apply_reader_pragmas(&c)?;
            let (q, codec, cache) = (jobs_rx.clone(), codec.clone(), cache_kib.clone());
            readers.push(
                Builder::new()
                    .name("parla-store-read".into())
                    .spawn(move || {
                        let mut applied = 0;
                        loop {
                            let job = q.lock().unwrap().recv();
                            let Ok(job) = job else { break };
                            let want = cache.load(Ordering::Relaxed);
                            if want != applied {
                                conn::set_cache_kib(&c, want);
                                applied = want;
                            }
                            job(&c, &codec);
                        }
                    })
                    .map_err(io)?,
            );
        }
        if let Some(s) = self.shared.upgrade() {
            s.readers.lock().unwrap().insert(ws.clone(), jobs_tx);
        }
        self.dbs.insert(
            ws.clone(),
            Db {
                writer,
                readers,
                cache_kib,
            },
        );
        self.rebalance();
        Ok(())
    }

    fn close(&mut self, ws: &DbId) {
        if let Some(s) = self.shared.upgrade() {
            s.readers.lock().unwrap().remove(ws);
        }
        if let Some(db) = self.dbs.remove(ws) {
            for h in db.readers {
                let _ = h.join();
            }
            drop(db.writer); // last connection closed: WAL checkpointed
        }
        self.rebalance();
    }

    fn rebuild(&mut self, ws: &DbId) -> Result<(), StoreError> {
        let codec = self.codecs.get(ws).cloned().ok_or(StoreError::Closed)?;
        self.close(ws);
        self.delete_files(ws)?;
        block_on(self.keys.replace(ws))?;
        self.open(ws, codec)
    }

    fn remove(&mut self, ws: &DbId) -> Result<(), StoreError> {
        self.close(ws);
        self.codecs.remove(ws);
        self.delete_files(ws)?;
        block_on(self.keys.delete(ws))
    }

    fn delete_files(&self, ws: &DbId) -> Result<(), StoreError> {
        let path = db_path(&self.cfg.dir, ws);
        for suffix in ["", "-wal", "-shm"] {
            let mut p = path.clone().into_os_string();
            p.push(suffix);
            match std::fs::remove_file(&p) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(crate::err::other(e)),
            }
        }
        Ok(())
    }

    /// `cache_size = -(cache_mb * 1024 / open_count)` KiB on every open database.
    fn rebalance(&self) {
        if self.dbs.is_empty() {
            return;
        }
        let kib = i64::from(self.cfg.cache_mb) * 1024 / self.dbs.len() as i64;
        for db in self.dbs.values() {
            db.cache_kib.store(kib, Ordering::Relaxed);
            conn::set_cache_kib(&db.writer, kib);
        }
    }
}

//! What a plugin may import: the
//! `parla:plugin` host interfaces plus the WASI interfaces every Rust std
//! guest imports (M8 A1 spike). No filesystem, no sockets, no `wasi:http`.
//! Stdin and the environment are empty, the terminal calls report no
//! terminal, and stdout and stderr go into the plugin's log.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll};

use wasmtime::Engine;
use wasmtime::component::{Component, Linker, ResourceTable};
use wasmtime_wasi::cli::{IsTerminal, StdoutStream};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_io::bytes::Bytes;
use wasmtime_wasi_io::streams::{OutputStream, StreamResult};

use crate::limits::MemoryLimiter;

/// Interfaces of the `parla:plugin` package a plugin may import. The host
/// functions themselves are linked by the host API tasks (plan M8 C2 to C4).
pub const PLUGIN_PACKAGE: &str = "parla:plugin/";

/// WASI 0.2 interfaces a plugin may import, without the `@version`.
pub const WASI_ALLOWED: &[&str] = &[
    "wasi:io/error",
    "wasi:io/poll",
    "wasi:io/streams",
    "wasi:clocks/monotonic-clock",
    "wasi:clocks/wall-clock",
    "wasi:random/random",
    "wasi:random/insecure",
    "wasi:random/insecure-seed",
    "wasi:cli/stdin",
    "wasi:cli/stdout",
    "wasi:cli/stderr",
    "wasi:cli/environment",
    "wasi:cli/exit",
    "wasi:cli/terminal-input",
    "wasi:cli/terminal-output",
    "wasi:cli/terminal-stdin",
    "wasi:cli/terminal-stdout",
    "wasi:cli/terminal-stderr",
];

/// Host resources (streams, pollables) one instance may hold.
pub const MAX_RESOURCES: usize = 1_000;

/// Bytes of stdout and stderr kept per instance; older output is dropped.
pub const LOG_CAPACITY: usize = 64 * 1024;

/// True when a component may import `name` (`wasi:io/poll@0.2.12`, ...).
pub fn import_allowed(name: &str) -> bool {
    if name.starts_with(PLUGIN_PACKAGE) {
        return true;
    }
    let base = name.split_once('@').map_or(name, |(base, _)| base);
    let version = name.split_once('@').map(|(_, v)| v);
    WASI_ALLOWED.contains(&base) && version.is_none_or(|v| v.starts_with("0.2."))
}

/// The first import of `component` that is not allowed.
pub fn forbidden_import(engine: &Engine, component: &Component) -> Option<String> {
    component
        .component_type()
        .imports(engine)
        .map(|(name, _)| name)
        .find(|name| !import_allowed(name))
        .map(str::to_owned)
}

/// Store data of one plugin instance.
pub struct PluginState {
    wasi: WasiCtx,
    table: ResourceTable,
    pub(crate) limiter: MemoryLimiter,
    log: LogPipe,
}

impl PluginState {
    pub fn new(limiter: MemoryLimiter) -> Self {
        let log = LogPipe::default();
        // Empty stdin, environment, arguments and cwd; no preopened
        // directories and no network (neither is linked anyway).
        let wasi = WasiCtxBuilder::new()
            .stdout(log.clone())
            .stderr(log.clone())
            .build();
        let mut table = ResourceTable::new();
        table.set_max_capacity(MAX_RESOURCES);
        PluginState {
            wasi,
            table,
            limiter,
            log,
        }
    }

    pub fn limiter(&self) -> &MemoryLimiter {
        &self.limiter
    }

    /// The instance's stdout and stderr, at most [`LOG_CAPACITY`] bytes.
    pub fn log(&self) -> String {
        self.log.contents()
    }
}

impl WasiView for PluginState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl wasmtime_wasi_io::IoView for PluginState {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

/// The linker every plugin instance uses: the allowed WASI interfaces only.
pub fn linker(engine: &Engine) -> wasmtime::Result<Linker<PluginState>> {
    use wasmtime_wasi::cli::{WasiCli, WasiCliView};
    use wasmtime_wasi::clocks::{WasiClocks, WasiClocksView};
    use wasmtime_wasi::p2::bindings::{cli, clocks, random};
    use wasmtime_wasi::random::WasiRandom;

    type S = PluginState;
    let mut l = Linker::new(engine);
    wasmtime_wasi_io::add_to_linker_async(&mut l)?;
    clocks::wall_clock::add_to_linker::<S, WasiClocks>(&mut l, S::clocks)?;
    clocks::monotonic_clock::add_to_linker::<S, WasiClocks>(&mut l, S::clocks)?;
    random::random::add_to_linker::<S, WasiRandom>(&mut l, |t| t.wasi.random())?;
    random::insecure::add_to_linker::<S, WasiRandom>(&mut l, |t| t.wasi.random())?;
    random::insecure_seed::add_to_linker::<S, WasiRandom>(&mut l, |t| t.wasi.random())?;
    cli::environment::add_to_linker::<S, WasiCli>(&mut l, S::cli)?;
    cli::exit::add_to_linker::<S, WasiCli>(&mut l, S::cli)?;
    cli::stdin::add_to_linker::<S, WasiCli>(&mut l, S::cli)?;
    cli::stdout::add_to_linker::<S, WasiCli>(&mut l, S::cli)?;
    cli::stderr::add_to_linker::<S, WasiCli>(&mut l, S::cli)?;
    cli::terminal_input::add_to_linker::<S, WasiCli>(&mut l, S::cli)?;
    cli::terminal_output::add_to_linker::<S, WasiCli>(&mut l, S::cli)?;
    cli::terminal_stdin::add_to_linker::<S, WasiCli>(&mut l, S::cli)?;
    cli::terminal_stdout::add_to_linker::<S, WasiCli>(&mut l, S::cli)?;
    cli::terminal_stderr::add_to_linker::<S, WasiCli>(&mut l, S::cli)?;
    Ok(l)
}

/// Bounded stdout and stderr sink: keeps the last [`LOG_CAPACITY`] bytes and
/// never blocks or fails the guest.
#[derive(Clone, Default)]
struct LogPipe {
    buf: Arc<Mutex<VecDeque<u8>>>,
}

impl LogPipe {
    fn push(&self, bytes: &[u8]) {
        let mut buf = self.buf.lock().unwrap_or_else(PoisonError::into_inner);
        let bytes = &bytes[bytes.len().saturating_sub(LOG_CAPACITY)..];
        let overflow = (buf.len() + bytes.len()).saturating_sub(LOG_CAPACITY);
        buf.drain(..overflow);
        buf.extend(bytes);
    }

    fn contents(&self) -> String {
        let buf = self.buf.lock().unwrap_or_else(PoisonError::into_inner);
        let (a, b) = buf.as_slices();
        let mut v = Vec::with_capacity(buf.len());
        v.extend_from_slice(a);
        v.extend_from_slice(b);
        String::from_utf8_lossy(&v).into_owned()
    }
}

impl IsTerminal for LogPipe {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl StdoutStream for LogPipe {
    fn p2_stream(&self) -> Box<dyn OutputStream> {
        Box::new(self.clone())
    }

    fn async_stream(&self) -> Box<dyn tokio::io::AsyncWrite + Send + Sync> {
        Box::new(self.clone())
    }
}

impl OutputStream for LogPipe {
    fn write(&mut self, bytes: Bytes) -> StreamResult<()> {
        self.push(&bytes);
        Ok(())
    }

    fn flush(&mut self) -> StreamResult<()> {
        Ok(())
    }

    fn check_write(&mut self) -> StreamResult<usize> {
        Ok(LOG_CAPACITY)
    }
}

#[wasmtime_wasi_io::async_trait]
impl wasmtime_wasi_io::poll::Pollable for LogPipe {
    async fn ready(&mut self) {}
}

impl tokio::io::AsyncWrite for LogPipe {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        self.push(buf);
        Poll::Ready(Ok(buf.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resource_table_is_capped() {
        let state = PluginState::new(MemoryLimiter::new(None));
        assert_eq!(state.table.max_capacity(), MAX_RESOURCES);
    }

    #[test]
    fn allow_list() {
        assert!(import_allowed("wasi:io/poll@0.2.12"));
        assert!(import_allowed("wasi:cli/terminal-stderr@0.2.0"));
        assert!(import_allowed("parla:plugin/host@0.1.0"));
        assert!(!import_allowed("wasi:filesystem/types@0.2.12"));
        assert!(!import_allowed("wasi:filesystem/preopens@0.2.12"));
        assert!(!import_allowed("wasi:sockets/tcp@0.2.12"));
        assert!(!import_allowed("wasi:http/outgoing-handler@0.2.12"));
        assert!(!import_allowed("wasi:io/poll@0.3.0"));
        assert!(!import_allowed("other:pkg/iface"));
    }

    #[test]
    fn log_keeps_the_newest_bytes() {
        let log = LogPipe::default();
        log.push(&vec![b'a'; LOG_CAPACITY]);
        log.push(b"tail");
        let s = log.contents();
        assert_eq!(s.len(), LOG_CAPACITY);
        assert!(s.ends_with("aaaatail"));
        log.push(&vec![b'b'; LOG_CAPACITY + 10]);
        assert_eq!(log.contents(), "b".repeat(LOG_CAPACITY));
    }
}

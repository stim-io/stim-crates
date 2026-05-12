//! `SidecarRuntime` — the unified socket fact every sidecar exposes
//! for the dev-loop control plane.
//!
//! The contract this module embodies is intentionally tiny:
//!
//! 1. On Unix platforms each sidecar binds a local Unix domain
//!    socket and publishes it as `unix:///absolute/path.sock` via
//!    its ready line's `runtime_endpoint` field. Non-Unix platforms
//!    keep a TCP fallback as `tcp://127.0.0.1:<port>`.
//! 2. The external sidecar CLI (or any other coordinator) connects to that endpoint
//!    and speaks NDJSON-framed control plane messages:
//!      - `{"kind":"ping","seq":N}` ↔ `{"kind":"pong","seq":N}`
//!      - `{"kind":"event","id":"<uuid>","verb":"<name>","payload":...}` ↔
//!        `{"kind":"event_response","id":"<uuid>","payload":...}` or
//!        `{"kind":"event_error","id":"<uuid>","error":{"code":"...","message":"..."}}`
//!
//! `SidecarRuntime` owns the listener + frame loop; the consuming
//! sidecar provides an `EventHandler` for non-heartbeat verbs. Any
//! verb the handler doesn't know returns `not_implemented` — by
//! design, manifest schemas don't enumerate sidecar capabilities;
//! discovery is by trial.
//!
//! What flows on top of this contract beyond heartbeat + events
//! is application protocol and stays outside the sidecar
//! contract. agents/controller continue to serve their HTTP product
//! API on a separate listener (published as the existing `endpoint`
//! field); Tauri host serves only the sidecar runtime.

use std::{
    env,
    future::Future,
    io,
    pin::Pin,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc,
    },
};

#[cfg(unix)]
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpListener,
};

#[cfg(unix)]
use tokio::net::UnixListener;

const INSPECT_SOCKET_ENV: &str = "SIDECAR_INSPECT_SOCKET";

/// Frame `kind` discriminator strings.
pub const FRAME_KIND_PING: &str = "ping";
pub const FRAME_KIND_PONG: &str = "pong";
pub const FRAME_KIND_EVENT: &str = "event";
pub const FRAME_KIND_EVENT_RESPONSE: &str = "event_response";
pub const FRAME_KIND_EVENT_ERROR: &str = "event_error";

/// Standard error code used when a sidecar's event handler does
/// not recognize the requested verb. Callers should treat this as
/// "capability not implemented for this sidecar".
pub const EVENT_ERROR_NOT_IMPLEMENTED: &str = "not_implemented";

/// All NDJSON frames are tagged by `kind`. Other frame variants
/// are deliberately denoted by separate structs in this module
/// rather than a single sum type so consuming code only depends
/// on the variants it cares about.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Frame {
    Ping {
        seq: u64,
    },
    Pong {
        seq: u64,
    },
    Event {
        id: String,
        verb: String,
        #[serde(default)]
        payload: Value,
    },
    EventResponse {
        id: String,
        #[serde(default)]
        payload: Value,
    },
    EventError {
        id: String,
        error: EventError,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EventError {
    pub code: String,
    pub message: String,
}

impl EventError {
    pub fn not_implemented(verb: &str) -> Self {
        Self {
            code: EVENT_ERROR_NOT_IMPLEMENTED.to_string(),
            message: format!("verb {verb:?} is not implemented"),
        }
    }

    pub fn invalid_payload(message: impl Into<String>) -> Self {
        Self {
            code: "invalid_payload".to_string(),
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal".to_string(),
            message: message.into(),
        }
    }
}

/// Result returned by an `EventHandler`. `Ok(payload)` becomes an
/// `event_response` frame; `Err(error)` becomes an `event_error`
/// frame.
pub type EventResult = std::result::Result<Value, EventError>;

/// Trait every sidecar implements to dispatch event-trigger verbs
/// it understands. Async-friendly: the future is boxed so the
/// trait stays object-safe under common async-trait patterns.
pub trait EventHandler: Send + Sync + 'static {
    fn handle<'a>(
        &'a self,
        verb: &'a str,
        payload: &'a Value,
    ) -> Pin<Box<dyn Future<Output = EventResult> + Send + 'a>>;
}

/// Convenience adapter: build an `EventHandler` from a closure.
pub struct ClosureHandler<F>(F);

impl<F, Fut> ClosureHandler<F>
where
    F: Fn(String, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = EventResult> + Send + 'static,
{
    pub fn new(f: F) -> Self {
        Self(f)
    }
}

impl<F, Fut> EventHandler for ClosureHandler<F>
where
    F: Fn(String, Value) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = EventResult> + Send + 'static,
{
    fn handle<'a>(
        &'a self,
        verb: &'a str,
        payload: &'a Value,
    ) -> Pin<Box<dyn Future<Output = EventResult> + Send + 'a>> {
        let fut = (self.0)(verb.to_string(), payload.clone());
        Box::pin(fut)
    }
}

/// Listener returned by [`bind`]. Unix platforms use a Unix domain
/// socket; non-Unix platforms use the TCP fallback.
pub enum RuntimeListener {
    #[cfg(unix)]
    Unix {
        endpoint: String,
        path: PathBuf,
        listener: UnixListener,
    },
    Tcp {
        endpoint: String,
        listener: TcpListener,
    },
}

impl RuntimeListener {
    pub fn endpoint(&self) -> &str {
        match self {
            #[cfg(unix)]
            Self::Unix { endpoint, .. } => endpoint,
            Self::Tcp { endpoint, .. } => endpoint,
        }
    }
}

/// Bind the platform runtime socket and return both the endpoint
/// string to publish in the ready line and the listener for
/// [`serve`].
pub async fn bind() -> std::io::Result<(String, RuntimeListener)> {
    if let Some(endpoint) = env::var(INSPECT_SOCKET_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
    {
        return bind_configured(&endpoint).await;
    }

    #[cfg(unix)]
    {
        let path = runtime_socket_path();
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        let endpoint = format!("unix://{}", path.display());
        Ok((
            endpoint.clone(),
            RuntimeListener::Unix {
                endpoint,
                path,
                listener,
            },
        ))
    }

    #[cfg(not(unix))]
    {
        bind_tcp().await
    }
}

async fn bind_configured(endpoint: &str) -> std::io::Result<(String, RuntimeListener)> {
    #[cfg(unix)]
    if let Some(path) = endpoint.strip_prefix("unix://") {
        if path.is_empty() || !path.starts_with('/') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "SIDECAR_INSPECT_SOCKET unix endpoint must use unix:///absolute/path.sock",
            ));
        }
        let path = PathBuf::from(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path)?;
        let endpoint = format!("unix://{}", path.display());
        return Ok((
            endpoint.clone(),
            RuntimeListener::Unix {
                endpoint,
                path,
                listener,
            },
        ));
    }

    if let Some(address) = endpoint.strip_prefix("tcp://") {
        let listener = TcpListener::bind(address).await?;
        let addr = listener.local_addr()?;
        let endpoint = format!("tcp://{addr}");
        return Ok((
            endpoint.clone(),
            RuntimeListener::Tcp { endpoint, listener },
        ));
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "SIDECAR_INSPECT_SOCKET must use unix:///absolute/path.sock or tcp://host:port",
    ))
}

#[cfg(not(unix))]
async fn bind_tcp() -> std::io::Result<(String, RuntimeListener)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    let endpoint = format!("tcp://{addr}");
    Ok((
        endpoint.clone(),
        RuntimeListener::Tcp { endpoint, listener },
    ))
}

#[cfg(unix)]
fn runtime_socket_path() -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    PathBuf::from("/tmp").join(format!(
        "stim-sidecar-runtime-{}-{counter}.sock",
        std::process::id()
    ))
}

/// Run the accept-and-dispatch loop on `listener`, forwarding
/// non-heartbeat events to `handler`. Returns when the listener
/// closes (typically a Tokio shutdown).
pub async fn serve<H: EventHandler>(listener: RuntimeListener, handler: H) -> std::io::Result<()> {
    match listener {
        #[cfg(unix)]
        RuntimeListener::Unix { listener, path, .. } => {
            let result = serve_unix(listener, handler).await;
            let _ = std::fs::remove_file(path);
            result
        }
        RuntimeListener::Tcp { listener, .. } => serve_tcp(listener, handler).await,
    }
}

async fn serve_tcp<H: EventHandler>(listener: TcpListener, handler: H) -> std::io::Result<()> {
    let handler = Arc::new(handler);
    loop {
        let (stream, _peer) = match listener.accept().await {
            Ok(v) => v,
            Err(error) => return Err(error),
        };
        let h = Arc::clone(&handler);
        tokio::spawn(async move {
            if let Err(error) = handle_connection(stream, h).await {
                eprintln!("sidecar runtime connection error: {error}");
            }
        });
    }
}

#[cfg(unix)]
async fn serve_unix<H: EventHandler>(listener: UnixListener, handler: H) -> std::io::Result<()> {
    let handler = Arc::new(handler);
    loop {
        let (stream, _peer) = match listener.accept().await {
            Ok(v) => v,
            Err(error) => return Err(error),
        };
        let h = Arc::clone(&handler);
        tokio::spawn(async move {
            if let Err(error) = handle_connection(stream, h).await {
                eprintln!("sidecar runtime connection error: {error}");
            }
        });
    }
}

async fn handle_connection<H, S>(stream: S, handler: Arc<H>) -> std::io::Result<()>
where
    H: EventHandler,
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (read_half, mut write_half) = tokio::io::split(stream);
    let mut reader = BufReader::new(read_half).lines();
    while let Some(line) = reader.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let frame: Frame = match serde_json::from_str(&line) {
            Ok(frame) => frame,
            Err(error) => {
                let err_frame = Frame::EventError {
                    id: String::new(),
                    error: EventError::invalid_payload(format!("invalid frame: {error}")),
                };
                write_frame(&mut write_half, &err_frame).await?;
                continue;
            }
        };
        let response = dispatch(&handler, frame).await;
        if let Some(out) = response {
            write_frame(&mut write_half, &out).await?;
        }
    }
    Ok(())
}

async fn dispatch<H: EventHandler>(handler: &Arc<H>, frame: Frame) -> Option<Frame> {
    match frame {
        Frame::Ping { seq } => Some(Frame::Pong { seq }),
        Frame::Event { id, verb, payload } => match handler.handle(&verb, &payload).await {
            Ok(payload) => Some(Frame::EventResponse { id, payload }),
            Err(error) => Some(Frame::EventError { id, error }),
        },
        // Sidecars ignore client-shaped frames; clients that want
        // sidecar-shaped frames talk to a different peer.
        Frame::Pong { .. } | Frame::EventResponse { .. } | Frame::EventError { .. } => None,
    }
}

async fn write_frame<W>(writer: &mut W, frame: &Frame) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mut bytes = serde_json::to_vec(frame).map_err(|error| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("frame serialize: {error}"),
        )
    })?;
    bytes.push(b'\n');
    writer.write_all(&bytes).await?;
    writer.flush().await
}

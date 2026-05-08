//! `SidecarRuntime` — the unified socket fact every sidecar exposes
//! for the dev-loop control plane.
//!
//! The contract this module embodies is intentionally tiny:
//!
//! 1. Each sidecar binds a TCP listener on `127.0.0.1:0` (OS picks
//!    a free port) and publishes the address via its ready line's
//!    new `runtime_endpoint` field.
//! 2. stim-dev (or any other coordinator) connects to that endpoint
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

use std::{future::Future, net::SocketAddr, pin::Pin, sync::Arc};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpListener, TcpStream},
};

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

/// Bind a TCP listener on `127.0.0.1:0` and return both the
/// listener (for `serve`) and the bound socket address (to publish
/// via the ready line's `runtime_endpoint` field).
pub async fn bind() -> std::io::Result<(SocketAddr, TcpListener)> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let addr = listener.local_addr()?;
    Ok((addr, listener))
}

/// Run the accept-and-dispatch loop on `listener`, forwarding
/// non-heartbeat events to `handler`. Returns when the listener
/// closes (typically a Tokio shutdown).
pub async fn serve<H: EventHandler>(listener: TcpListener, handler: H) -> std::io::Result<()> {
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

async fn handle_connection<H: EventHandler>(
    stream: TcpStream,
    handler: Arc<H>,
) -> std::io::Result<()> {
    let (read_half, mut write_half) = stream.into_split();
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

async fn write_frame(
    writer: &mut tokio::net::tcp::OwnedWriteHalf,
    frame: &Frame,
) -> std::io::Result<()> {
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

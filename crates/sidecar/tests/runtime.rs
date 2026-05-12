use std::{future::Future, pin::Pin};

use serde_json::Value;
use stim_sidecar::runtime::{
    bind, serve, ClosureHandler, EventError, EventResult, Frame, EVENT_ERROR_NOT_IMPLEMENTED,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader},
    net::TcpStream,
};

#[cfg(unix)]
use tokio::net::UnixStream;

type EventFuture = Pin<Box<dyn Future<Output = EventResult> + Send + 'static>>;
type EventFn = Box<dyn Fn(String, Value) -> EventFuture + Send + Sync + 'static>;

fn handler() -> ClosureHandler<EventFn> {
    let f: EventFn = Box::new(|verb: String, payload: Value| {
        Box::pin(async move {
            match verb.as_str() {
                "echo" => Ok(payload),
                "boom" => Err(EventError::internal("boom".to_string())),
                _ => Err(EventError::not_implemented(&verb)),
            }
        }) as EventFuture
    });
    ClosureHandler::new(f)
}

async fn round_trip(req: &str) -> String {
    let (endpoint, listener) = bind().await.unwrap();
    assert_runtime_endpoint_shape(&endpoint);
    let server = tokio::spawn(async move { serve(listener, handler()).await.unwrap_or(()) });

    let mut stream = connect_runtime(&endpoint).await;
    let request = format!("{req}\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    let mut response = String::new();
    let (read, _write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read);
    reader.read_line(&mut response).await.unwrap();
    server.abort();
    response.trim().to_string()
}

#[cfg(unix)]
fn assert_runtime_endpoint_shape(endpoint: &str) {
    assert!(endpoint.starts_with("unix:///"), "{endpoint}");
}

#[cfg(not(unix))]
fn assert_runtime_endpoint_shape(endpoint: &str) {
    assert!(endpoint.starts_with("tcp://"), "{endpoint}");
}

async fn connect_runtime(endpoint: &str) -> impl AsyncRead + AsyncWrite + Unpin {
    #[cfg(unix)]
    if let Some(path) = endpoint.strip_prefix("unix://") {
        return RuntimeStream::Unix(UnixStream::connect(path).await.unwrap());
    }

    let address = endpoint.strip_prefix("tcp://").unwrap_or(endpoint);
    RuntimeStream::Tcp(TcpStream::connect(address).await.unwrap())
}

enum RuntimeStream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl AsyncRead for RuntimeStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        match &mut *self {
            Self::Tcp(stream) => Pin::new(stream).poll_read(cx, buf),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for RuntimeStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buf: &[u8],
    ) -> std::task::Poll<Result<usize, std::io::Error>> {
        match &mut *self {
            Self::Tcp(stream) => Pin::new(stream).poll_write(cx, buf),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_write(cx, buf),
        }
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        match &mut *self {
            Self::Tcp(stream) => Pin::new(stream).poll_flush(cx),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_flush(cx),
        }
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), std::io::Error>> {
        match &mut *self {
            Self::Tcp(stream) => Pin::new(stream).poll_shutdown(cx),
            #[cfg(unix)]
            Self::Unix(stream) => Pin::new(stream).poll_shutdown(cx),
        }
    }
}

mod ping {
    use super::*;

    #[tokio::test]
    async fn returns_matching_pong() {
        let resp = round_trip(r#"{"kind":"ping","seq":42}"#).await;
        let parsed: Frame = serde_json::from_str(&resp).expect("parse");
        assert_eq!(parsed, Frame::Pong { seq: 42 });
    }
}

mod event {
    use super::*;

    #[tokio::test]
    async fn dispatches_response() {
        let resp =
            round_trip(r#"{"kind":"event","id":"req-1","verb":"echo","payload":{"hi":1}}"#).await;
        let parsed: Frame = serde_json::from_str(&resp).expect("parse");
        match parsed {
            Frame::EventResponse { id, payload } => {
                assert_eq!(id, "req-1");
                assert_eq!(payload, serde_json::json!({"hi": 1}));
            }
            other => panic!("expected EventResponse, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn reports_unknown_verb() {
        let resp =
            round_trip(r#"{"kind":"event","id":"req-2","verb":"missing","payload":{}}"#).await;
        let parsed: Frame = serde_json::from_str(&resp).expect("parse");
        match parsed {
            Frame::EventError { id, error } => {
                assert_eq!(id, "req-2");
                assert_eq!(error.code, EVENT_ERROR_NOT_IMPLEMENTED);
            }
            other => panic!("expected EventError, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn propagates_internal_error() {
        let resp = round_trip(r#"{"kind":"event","id":"req-3","verb":"boom","payload":{}}"#).await;
        let parsed: Frame = serde_json::from_str(&resp).expect("parse");
        match parsed {
            Frame::EventError { id, error } => {
                assert_eq!(id, "req-3");
                assert_eq!(error.code, "internal");
                assert_eq!(error.message, "boom");
            }
            other => panic!("expected EventError, got {other:?}"),
        }
    }
}

mod invalid_frame {
    use super::*;

    #[tokio::test]
    async fn returns_payload_error() {
        let resp = round_trip(r#"{"not":"a frame"}"#).await;
        let parsed: Frame = serde_json::from_str(&resp).expect("parse");
        match parsed {
            Frame::EventError { id, error } => {
                assert_eq!(id, "");
                assert_eq!(error.code, "invalid_payload");
            }
            other => panic!("expected EventError, got {other:?}"),
        }
    }
}

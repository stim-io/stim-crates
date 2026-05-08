use std::{future::Future, pin::Pin};

use serde_json::Value;
use stim_sidecar::runtime::{
    bind, serve, ClosureHandler, EventError, EventResult, Frame, EVENT_ERROR_NOT_IMPLEMENTED,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::TcpStream,
};

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
    let (addr, listener) = bind().await.unwrap();
    let server = tokio::spawn(async move { serve(listener, handler()).await.unwrap_or(()) });

    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!("{req}\n");
    stream.write_all(request.as_bytes()).await.unwrap();
    stream.flush().await.unwrap();

    let mut response = String::new();
    let (read, _write) = stream.into_split();
    let mut reader = BufReader::new(read);
    reader.read_line(&mut response).await.unwrap();
    server.abort();
    response.trim().to_string()
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

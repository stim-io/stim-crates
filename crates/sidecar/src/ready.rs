use std::{
    fmt,
    io::{BufRead, BufReader, Read},
    sync::mpsc,
    time::Duration,
};

use serde::{Deserialize, Serialize};

use crate::identity::SidecarStamp;

pub const READY_LINE_KIND: &str = "stim-sidecar-ready";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SidecarReadyLine {
    pub kind: String,
    pub stamp: SidecarStamp,
    pub role: String,
    pub instance_id: String,
    /// Business endpoint the sidecar exposes to product consumers
    /// (e.g. agents/controller's HTTP base URL). Stays unchanged.
    pub endpoint: Option<String>,
    /// Endpoint of the sidecar's `SidecarRuntime` listener — the
    /// unified socket fact external sidecar uses for heartbeat + event
    /// triggers. Sidecars that don't yet adopt SidecarRuntime
    /// leave this as None; older sidecars without the field deserialize
    /// to None (the field is `serde(default)`).
    #[serde(default)]
    pub runtime_endpoint: Option<String>,
    pub ready_at: String,
}

impl SidecarReadyLine {
    pub fn new(
        stamp: SidecarStamp,
        role: String,
        instance_id: String,
        endpoint: Option<String>,
        ready_at: String,
    ) -> Self {
        Self {
            kind: READY_LINE_KIND.to_string(),
            stamp,
            role,
            instance_id,
            endpoint,
            runtime_endpoint: None,
            ready_at,
        }
    }

    pub fn with_runtime_endpoint(mut self, runtime_endpoint: impl Into<String>) -> Self {
        self.runtime_endpoint = Some(runtime_endpoint.into());
        self
    }

    pub fn is_ready_line(&self) -> bool {
        self.kind == READY_LINE_KIND
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadyLineWaitError {
    ExitedBeforeReady,
    ReadFailed(String),
    TimedOut,
}

impl fmt::Display for ReadyLineWaitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ExitedBeforeReady => formatter.write_str("sidecar exited before ready"),
            Self::ReadFailed(error) => {
                write!(formatter, "failed to read sidecar ready line: {error}")
            }
            Self::TimedOut => formatter.write_str("timed out waiting for sidecar ready line"),
        }
    }
}

impl std::error::Error for ReadyLineWaitError {}

pub fn wait_for_ready_line<R>(
    reader: R,
    timeout: Duration,
) -> Result<SidecarReadyLine, ReadyLineWaitError>
where
    R: Read + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();

    std::thread::spawn(move || {
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) => {
                    let _ = sender.send(Err(ReadyLineWaitError::ExitedBeforeReady));
                    return;
                }
                Ok(_) => {
                    if let Ok(ready) = serde_json::from_str::<SidecarReadyLine>(line.trim()) {
                        let _ = sender.send(Ok(ready));
                        drain_remaining_lines(reader);
                        return;
                    }
                }
                Err(error) => {
                    let _ = sender.send(Err(ReadyLineWaitError::ReadFailed(error.to_string())));
                    return;
                }
            }
        }
    });

    receiver
        .recv_timeout(timeout)
        .map_err(|_| ReadyLineWaitError::TimedOut)?
}

fn drain_remaining_lines<R>(mut reader: BufReader<R>)
where
    R: Read,
{
    let mut line = String::new();
    while reader.read_line(&mut line).is_ok_and(|read| read > 0) {
        line.clear();
    }
}

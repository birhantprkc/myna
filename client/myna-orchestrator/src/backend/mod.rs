//! The inference-backend boundary — the seam between the orchestrator FSM and
//! the STT service (the inference snap, or the Python `myna-server` standing in
//! for it during development).
//!
//! The FSM never touches a socket: it drives a [`BackendClient`], which yields a
//! [`BackendHandle`] split into a cheap-clone [`BackendSink`] (audio + control
//! *up*) and a [`BackendEvents`] receiver (transcript events *down*). That split
//! is what lets the FSM push audio and consume events **concurrently** over one
//! session, and it decouples the FSM from the wire entirely: the WS-over-UDS
//! client ([`ws_unix`]) and the T40 fake backend implement the same trait over
//! the same channels.

pub mod fake;
pub mod share;
mod transport;
pub mod ws_unix;
pub mod ws_unix_ie115;

use std::fmt;
use std::sync::Arc;

use async_trait::async_trait;
use myna_core::{PcmChunk, SessionConfig, TranscriptionEvent, WireError};
use tokio::sync::{mpsc, watch};

use crate::i18n::tr;
use crate::task::TaskGuard;

/// A client that opens transcription sessions against an STT backend.
#[async_trait]
pub trait BackendClient: Send + Sync {
    /// Open a session: perform the handshake (declare the protocol version,
    /// send the config, await the `session.created` ack) and return a handle
    /// ready to stream audio and receive events. Fails if the backend rejects
    /// the session (e.g. unsupported protocol version) or can't be reached.
    async fn open_session(&self, config: SessionConfig) -> Result<BackendHandle, BackendError>;
}

/// What the FSM sends *up* to the backend over a session, in order. Mirrors
/// the client side of the ws+unix wire: PCM binary frames, then a
/// `session.finish` control frame at end-of-audio. Abort is not queued here:
/// it is out of band ([`BackendSink::abort`]).
#[derive(Debug)]
pub enum Outbound {
    /// A chunk of PCM to transcribe (goes out as a binary frame).
    Audio(PcmChunk),
    /// End of audio, hotkey released (`session.finish`). The backend keeps
    /// decoding the tail and finishes with a terminal event.
    Finish,
}

/// Failure interacting with the backend.
///
/// [`BackendError::headline`] is what the user is told, translated through
/// this crate's gettext domain ([`crate::i18n`]); `Display` is the untranslated
/// detail for logs and Diagnostics.
#[derive(Debug)]
pub enum BackendError {
    /// No single backend socket could be named (see [`share::resolve`]).
    Resolve(share::ResolveError),
    Connect(String),
    Handshake(String),
    /// The backend refused the session with a terminal error during the
    /// handshake (e.g. `unsupported_protocol_version`).
    Rejected {
        code: String,
        message: String,
    },
    Wire(WireError),
    Closed,
    Transport(String),
}

impl BackendError {
    /// The short, translated message the user sees.
    pub fn headline(&self) -> String {
        match self {
            BackendError::Resolve(e) => e.headline(),
            BackendError::Connect(_) => tr("Model not reachable"),
            BackendError::Handshake(_) => crate::failure::model_not_responding(),
            BackendError::Rejected { code, .. } if code == "unsupported_protocol_version" => {
                tr("Model not compatible")
            }
            BackendError::Rejected { .. } | BackendError::Wire(_) => tr("Model error"),
            BackendError::Closed => tr("Model stopped"),
            BackendError::Transport(_) => crate::failure::model_connection_lost(),
        }
    }

    /// The headline and the detail, as one [`Failure`](crate::failure::Failure).
    pub fn failure(&self) -> crate::failure::Failure {
        crate::failure::Failure::new(self.headline(), self.to_string())
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendError::Resolve(e) => write!(f, "{e}"),
            BackendError::Connect(inner) => write!(f, "cannot reach the model: {inner}"),
            BackendError::Handshake(inner) => write!(f, "handshake failed: {inner}"),
            BackendError::Rejected { code, message } => {
                write!(f, "session rejected: {code}: {message}")
            }
            BackendError::Wire(e) => write!(f, "malformed event from the model: {e}"),
            BackendError::Closed => write!(f, "the model closed the connection unexpectedly"),
            BackendError::Transport(inner) => write!(f, "transport error: {inner}"),
        }
    }
}

impl std::error::Error for BackendError {}

impl From<share::ResolveError> for BackendError {
    fn from(e: share::ResolveError) -> Self {
        BackendError::Resolve(e)
    }
}

impl From<WireError> for BackendError {
    fn from(e: WireError) -> Self {
        BackendError::Wire(e)
    }
}

/// The audio/control side of an open session. Cheap to clone (it is a channel
/// sender), so the FSM can hand a clone to an audio-pump task while it consumes
/// events elsewhere.
#[derive(Clone)]
pub struct BackendSink {
    tx: mpsc::Sender<Outbound>,
    abort: Arc<watch::Sender<bool>>,
}

impl BackendSink {
    /// Wait for room in the outbound queue without committing an item, so a
    /// caller can keep serving other work while the transport is congested.
    pub(crate) async fn reserve(&self) -> Result<mpsc::Permit<'_, Outbound>, BackendError> {
        self.tx.reserve().await.map_err(|_| BackendError::Closed)
    }

    /// Abort the session (close without finishing); nothing is committed.
    /// Never waits: it bypasses queued audio, so it works while the transport
    /// is congested.
    pub fn abort(&self) {
        self.abort.send_replace(true);
    }
}

/// The event side of an open session: transcript events flow down until a
/// terminal event ([`TranscriptionEvent::is_terminal`]) or the connection
/// closes (then `None`). Dropping it cancels the transport task feeding it.
pub struct BackendEvents {
    rx: mpsc::Receiver<Result<TranscriptionEvent, BackendError>>,
    activity: Option<watch::Receiver<()>>,
    _transport: Option<TaskGuard>,
}

impl BackendEvents {
    /// Await the next event. `None` means the stream ended (terminal event
    /// already delivered, or the connection closed).
    pub async fn next(&mut self) -> Option<Result<TranscriptionEvent, BackendError>> {
        self.rx.recv().await
    }

    /// Take the signal of backend activity; later calls get one that never
    /// fires.
    pub(crate) fn activity(&mut self) -> Activity {
        Activity(self.activity.take())
    }

    /// Report activity from `activity`, which the transport marks per data frame.
    pub(crate) fn watching(mut self, activity: watch::Receiver<()>) -> Self {
        self.activity = Some(activity);
        self
    }

    /// Tie `transport`'s lifetime to these events.
    pub(crate) fn owning(mut self, transport: TaskGuard) -> Self {
        self._transport = Some(transport);
        self
    }
}

/// Every data frame the backend sends, whether or not it decodes to an event.
pub(crate) struct Activity(Option<watch::Receiver<()>>);

impl Activity {
    /// Resolves once the backend has sent something since the last call;
    /// never, for a backend that reports no activity or has gone.
    pub(crate) async fn seen(&mut self) {
        let Some(frames) = &mut self.0 else {
            return std::future::pending().await;
        };
        if frames.changed().await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// Where a transport sends transcript events down to the FSM.
pub(crate) type EventSender = mpsc::Sender<Result<TranscriptionEvent, BackendError>>;

/// The transport's end of a session's outbound queue. The fields are apart so
/// a pump can wait on both at once.
pub(crate) struct Outbox {
    pub(crate) queue: mpsc::Receiver<Outbound>,
    pub(crate) abort: AbortSignal,
}

/// The receiving end of [`BackendSink::abort`].
pub(crate) struct AbortSignal(watch::Receiver<bool>);

impl AbortSignal {
    /// Resolves once the client aborts; never, if every sink goes away
    /// without aborting.
    pub(crate) async fn aborted(&mut self) {
        if self.0.wait_for(|aborted| *aborted).await.is_err() {
            std::future::pending::<()>().await;
        }
    }
}

/// The channels of one session: the client halves and the transport's ends.
pub(crate) fn channels(
    outbound_capacity: usize,
    event_capacity: usize,
) -> (BackendSink, Outbox, BackendEvents, EventSender) {
    let (out_tx, out_rx) = mpsc::channel(outbound_capacity);
    let (abort_tx, abort_rx) = watch::channel(false);
    let (ev_tx, ev_rx) = mpsc::channel(event_capacity);
    (
        BackendSink {
            tx: out_tx,
            abort: Arc::new(abort_tx),
        },
        Outbox {
            queue: out_rx,
            abort: AbortSignal(abort_rx),
        },
        BackendEvents {
            rx: ev_rx,
            activity: None,
            _transport: None,
        },
        ev_tx,
    )
}

/// A live session: the two halves plus the protocol version the backend
/// acknowledged in `session.created` (`None` from a pre-versioning peer).
pub struct BackendHandle {
    pub sink: BackendSink,
    pub events: BackendEvents,
    protocol_version: Option<String>,
}

impl BackendHandle {
    pub(crate) fn new(
        sink: BackendSink,
        events: BackendEvents,
        protocol_version: Option<String>,
    ) -> Self {
        Self {
            sink,
            events,
            protocol_version,
        }
    }

    pub fn protocol_version(&self) -> Option<&str> {
        self.protocol_version.as_deref()
    }

    /// Take the two halves apart for independent, concurrent use.
    pub fn split(self) -> (BackendSink, BackendEvents, Option<String>) {
        (self.sink, self.events, self.protocol_version)
    }
}

#[cfg(test)]
mod tests {
    use super::share::{ResolveError, Unusable};
    use super::BackendError;

    #[test]
    fn errors_render_their_details_untranslated() {
        assert_eq!(
            BackendError::Connect("no socket".into()).to_string(),
            "cannot reach the model: no socket"
        );
        assert_eq!(
            BackendError::Handshake("timeout".into()).to_string(),
            "handshake failed: timeout"
        );
        assert_eq!(
            BackendError::Rejected {
                code: "unsupported_protocol_version".into(),
                message: "want 2".into(),
            }
            .to_string(),
            "session rejected: unsupported_protocol_version: want 2"
        );
        assert_eq!(
            BackendError::Closed.to_string(),
            "the model closed the connection unexpectedly"
        );
        assert_eq!(
            BackendError::Transport("reset".into()).to_string(),
            "transport error: reset"
        );
        let resolve = ResolveError::Ambiguous(vec!["a".into(), "b".into()]);
        let detail = resolve.to_string();
        assert_eq!(BackendError::from(resolve).to_string(), detail);
    }

    #[test]
    fn every_error_has_its_headline() {
        let cases = [
            (
                BackendError::Resolve(ResolveError::NotConnected(Unusable::default())),
                "Model not connected",
            ),
            (BackendError::Connect("x".into()), "Model not reachable"),
            (BackendError::Handshake("x".into()), "Model not responding"),
            (
                BackendError::Rejected {
                    code: "unsupported_protocol_version".into(),
                    message: "want 2".into(),
                },
                "Model not compatible",
            ),
            (
                BackendError::Rejected {
                    code: "busy".into(),
                    message: "x".into(),
                },
                "Model error",
            ),
            (
                BackendError::Wire(myna_core::WireError::NotAnEvent),
                "Model error",
            ),
            (BackendError::Closed, "Model stopped"),
            (BackendError::Transport("x".into()), "Model connection lost"),
        ];
        for (error, headline) in cases {
            assert_eq!(error.headline(), headline, "{error:?}");
            let failure = error.failure();
            assert_eq!(failure.headline, headline);
            assert_eq!(failure.detail, error.to_string());
        }
    }
}

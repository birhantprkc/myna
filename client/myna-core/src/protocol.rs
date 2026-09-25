//! Wire-protocol versioning — the Rust mirror of Python `myna.core.protocol`.
//!
//! A single number versions the whole client↔service contract over a transport:
//! the handshake, the event vocabulary, and the config/capabilities wire shapes.
//! It travels in band in the opening `session.start` (transport-agnostic, not a
//! WebSocket subprotocol token). Adding or renaming any of those is a breaking
//! change — bump the version.

/// The protocol version this build speaks.
pub const PROTOCOL_VERSION: &str = "1";

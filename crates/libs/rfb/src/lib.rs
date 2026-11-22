//! RFB (RFC 6143), the protocol of VNC: messages, encodings, authentication and the viewer's
//! side of a connection, shared by Screen Sharing's server and its viewer.

pub mod auth;
pub mod client;
pub mod clipboard;
pub mod decode;
pub mod encode;
pub mod protocol;
pub mod transport;

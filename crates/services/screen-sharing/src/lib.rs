//! Screen Sharing: an RFB (RFC 6143) server that lets VNC viewers on the local network see and
//! control this screen, and what Settings needs to know about it.

#[cfg(feature = "service")]
pub mod rfb;
pub mod setup;

mod auth;
mod encode;
pub mod identity;
mod protocol;
mod screen;
mod session;

use std::any::Any;
use std::net::IpAddr;
use std::sync::Arc;

pub use encode::Frame;
pub use screen::Screen;
pub use session::serve;

/// Where a viewer's pointer and keys go.
pub trait Input: Send + Sync {
    /// The pointer moved to `x`, `y` in screen pixels with these RFB buttons held (bit 0 the
    /// primary button, bits 3 to 6 the scroll wheel).
    fn pointer(&self, x: u32, y: u32, buttons: u8);
    /// A key with this X keysym went down or up.
    fn key(&self, keysym: u32, down: bool);
}

/// The screen a viewer sees and the input it sends, kept going while any viewer holds it.
pub struct Viewing {
    pub screen: Arc<Screen>,
    pub input: Arc<dyn Input>,
    pub kept: Arc<dyn Any + Send + Sync>,
}

/// What the server asks of the session it runs in.
pub trait Host: Send + Sync {
    /// The password viewers must know, if one is set.
    fn password(&self) -> Option<String>;
    /// Whether viewers without TLS may connect with plain VNC authentication.
    fn older_viewers(&self) -> bool;
    /// Whether the viewer at `peer`, which knows the password, may see the screen.
    fn admit(&self, peer: IpAddr) -> bool;
    /// The viewer at `peer` sees the screen from now on.
    fn view(&self, peer: IpAddr) -> Option<Viewing>;
    /// The name viewers show for this screen.
    fn name(&self) -> String;
}

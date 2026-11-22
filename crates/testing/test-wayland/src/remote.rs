use std::os::fd::AsFd;
use std::os::unix::net::UnixStream;

use bande_protocols::client::bande_capture_v1::BandeCaptureV1;
use wayland_client::delegate_noop;

use crate::{Compositor, Globals, TestClient};

impl TestClient {
    /// Asks the compositor, as the screenshot tool does, to serve emulated input (EIS) on a
    /// socket, and returns the other end.
    pub fn connect_remote_input(&mut self, server: &mut Compositor) -> UnixStream {
        let capture: BandeCaptureV1 = self.bind("bande_capture_v1", 3);
        let (ours, theirs) = UnixStream::pair().expect("socket pair");
        capture.connect_input(theirs.as_fd());
        self.exchange(server);
        ours
    }
}

delegate_noop!(Globals: ignore BandeCaptureV1);

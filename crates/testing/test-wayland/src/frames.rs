use wayland_client::protocol::wl_callback::{self, WlCallback};
use wayland_client::protocol::wl_subcompositor::WlSubcompositor;
use wayland_client::protocol::wl_subsurface::WlSubsurface;
use wayland_client::{Connection, Dispatch, QueueHandle, delegate_noop};

use crate::{Globals, ShmImage, TestClient};

pub(crate) type Subsurface = WlSubsurface;

impl TestClient {
    pub fn ask_for_frame(&mut self, index: usize) {
        let surface = self.windows[index].0.clone();
        self.ask_for_frame_on(&surface);
    }

    pub fn add_child(&mut self, index: usize) -> usize {
        let qh = self.queue.handle();
        let parent = self.windows[index].0.clone();
        let compositor = self.globals.compositor.as_ref().expect("wl_compositor");
        let surface = compositor.create_surface(&qh, ());
        let subcompositor: WlSubcompositor = self.bind("wl_subcompositor", 1);
        let subsurface = subcompositor.get_subsurface(&surface, &parent, &qh, ());
        subsurface.set_desync();
        surface.commit();
        parent.commit();
        self.connection.flush().expect("client flush");
        self.children.push((surface, subsurface));
        self.children.len() - 1
    }

    pub fn ask_for_child_frame(&mut self, child: usize) {
        let surface = self.children[child].0.clone();
        self.ask_for_frame_on(&surface);
    }

    pub fn fill_child(&mut self, child: usize, image: &ShmImage) {
        let surface = self.children[child].0.clone();
        let buffer = self.shm_buffer(image);
        surface.attach(Some(&buffer), 0, 0);
        surface.damage_buffer(0, 0, image.width, image.height);
        surface.commit();
        self.connection.flush().expect("client flush");
    }

    fn ask_for_frame_on(&mut self, surface: &wayland_client::protocol::wl_surface::WlSurface) {
        surface.frame(&self.queue.handle(), ());
        surface.commit();
        self.connection.flush().expect("client flush");
    }

    pub fn frames_done(&mut self) -> u32 {
        self.connection.flush().expect("client flush");
        if let Some(guard) = self.connection.prepare_read() {
            let _ = guard.read();
        }
        self.queue
            .dispatch_pending(&mut self.globals)
            .expect("client dispatch");
        self.globals.frames_done
    }
}

impl Dispatch<WlCallback, ()> for Globals {
    fn event(
        globals: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event {
            globals.frames_done += 1;
        }
    }
}

delegate_noop!(Globals: WlSubcompositor);
delegate_noop!(Globals: ignore WlSubsurface);

use std::sync::Mutex;

use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel::XdgToplevel;
use smithay::reexports::wayland_protocols::xdg::toplevel_drag::v1::server::xdg_toplevel_drag_manager_v1::{
    self, XdgToplevelDragManagerV1,
};
use smithay::reexports::wayland_protocols::xdg::toplevel_drag::v1::server::xdg_toplevel_drag_v1::{
    self, XdgToplevelDragV1,
};
use smithay::reexports::wayland_server::protocol::wl_data_source::WlDataSource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{
    Client, DataInit, Dispatch, DisplayHandle, GlobalDispatch, New, Resource,
};
use smithay::utils::{Logical, Point};

use crate::state::State;

pub(crate) struct ToplevelDrag {
    source: WlDataSource,
    attached: Mutex<Option<(XdgToplevel, Point<i32, Logical>)>>,
}

impl ToplevelDrag {
    fn attached(&self) -> Option<(XdgToplevel, Point<i32, Logical>)> {
        self.attached
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }
}

impl State {
    pub(crate) fn dragged_window(&self) -> Option<(WlSurface, Point<i32, Logical>)> {
        let source = self.drag.as_ref()?.source()?;
        let (toplevel, offset) = self
            .toplevel_drags
            .iter()
            .filter_map(|drag| drag.data::<ToplevelDrag>())
            .find(|drag| drag.source == *source)?
            .attached()?;
        let window = self
            .windows
            .iter()
            .find(|window| *window.toplevel.xdg_toplevel() == toplevel)?;
        Some((window.surface().clone(), offset))
    }

    pub(crate) fn dragged_window_at(&self, surface: &WlSurface) -> Option<Point<i32, Logical>> {
        let (dragged, offset) = self.dragged_window()?;
        let pointer = self.pointer_location;
        (dragged == *surface)
            .then(|| Point::from((pointer.x.round() as i32, pointer.y.round() as i32)) - offset)
    }

    pub(crate) fn follow_drag_with_window(&mut self) {
        let Some((surface, _)) = self.dragged_window() else {
            return;
        };
        let mapped = self
            .window_index(&surface)
            .is_some_and(|index| self.windows[index].size.is_some());
        if let (true, Some(at)) = (mapped, self.dragged_window_at(&surface)) {
            self.move_window_to(&surface, at);
        }
    }

    pub(crate) fn drop_window_drag_on_nothing(&self) {
        let Some(drag) = self.drag.as_ref().filter(|drag| drag.action().is_empty()) else {
            return;
        };
        if self.dragged_window().is_none() {
            return;
        }
        if let Some(source) = drag.source().filter(|source| source.version() >= 3) {
            source.dnd_drop_performed();
        }
    }
}

impl GlobalDispatch<XdgToplevelDragManagerV1, ()> for State {
    fn bind(
        _state: &mut Self,
        _display: &DisplayHandle,
        _client: &Client,
        resource: New<XdgToplevelDragManagerV1>,
        _global_data: &(),
        data_init: &mut DataInit<'_, Self>,
    ) {
        data_init.init(resource, ());
    }
}

impl Dispatch<XdgToplevelDragManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        _manager: &XdgToplevelDragManagerV1,
        request: xdg_toplevel_drag_manager_v1::Request,
        _data: &(),
        _display: &DisplayHandle,
        data_init: &mut DataInit<'_, Self>,
    ) {
        if let xdg_toplevel_drag_manager_v1::Request::GetXdgToplevelDrag { id, data_source } =
            request
        {
            let drag = data_init.init(
                id,
                ToplevelDrag {
                    source: data_source,
                    attached: Mutex::new(None),
                },
            );
            state.toplevel_drags.retain(Resource::is_alive);
            state.toplevel_drags.push(drag);
        }
    }
}

impl Dispatch<XdgToplevelDragV1, ToplevelDrag> for State {
    fn request(
        state: &mut Self,
        _client: &Client,
        _drag: &XdgToplevelDragV1,
        request: xdg_toplevel_drag_v1::Request,
        data: &ToplevelDrag,
        _display: &DisplayHandle,
        _data_init: &mut DataInit<'_, Self>,
    ) {
        match request {
            xdg_toplevel_drag_v1::Request::Attach {
                toplevel,
                x_offset,
                y_offset,
            } => {
                *data
                    .attached
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) =
                    Some((toplevel, (x_offset, y_offset).into()));
                state.follow_drag_with_window();
                state.needs_redraw = true;
            }
            xdg_toplevel_drag_v1::Request::Destroy => {
                state.toplevel_drags.retain(Resource::is_alive);
            }
            _ => {}
        }
    }
}

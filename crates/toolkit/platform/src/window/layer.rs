use bande_graphics::Rect;
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use super::{ShellSurface, Window, WindowError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerKind {
    Background,
    Bottom,
    Top,
    Overlay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Edges {
    pub top: bool,
    pub bottom: bool,
    pub left: bool,
    pub right: bool,
}

impl Edges {
    pub const ALL: Self = Self {
        top: true,
        bottom: true,
        left: true,
        right: true,
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyboardFocus {
    None,
    OnDemand,
    Exclusive,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerOptions {
    pub layer: LayerKind,
    pub anchor: Edges,
    pub exclusive_zone: i32,
    pub keyboard: KeyboardFocus,
    pub namespace: String,
}

pub(super) fn layer_of(kind: LayerKind) -> zwlr_layer_shell_v1::Layer {
    match kind {
        LayerKind::Background => zwlr_layer_shell_v1::Layer::Background,
        LayerKind::Bottom => zwlr_layer_shell_v1::Layer::Bottom,
        LayerKind::Top => zwlr_layer_shell_v1::Layer::Top,
        LayerKind::Overlay => zwlr_layer_shell_v1::Layer::Overlay,
    }
}

pub(super) fn anchor_of(edges: Edges) -> zwlr_layer_surface_v1::Anchor {
    let mut anchor = zwlr_layer_surface_v1::Anchor::empty();
    for (on, edge) in [
        (edges.top, zwlr_layer_surface_v1::Anchor::Top),
        (edges.bottom, zwlr_layer_surface_v1::Anchor::Bottom),
        (edges.left, zwlr_layer_surface_v1::Anchor::Left),
        (edges.right, zwlr_layer_surface_v1::Anchor::Right),
    ] {
        if on {
            anchor |= edge;
        }
    }
    anchor
}

pub(super) fn keyboard_interactivity(
    focus: KeyboardFocus,
) -> zwlr_layer_surface_v1::KeyboardInteractivity {
    match focus {
        KeyboardFocus::None => zwlr_layer_surface_v1::KeyboardInteractivity::None,
        KeyboardFocus::OnDemand => zwlr_layer_surface_v1::KeyboardInteractivity::OnDemand,
        KeyboardFocus::Exclusive => zwlr_layer_surface_v1::KeyboardInteractivity::Exclusive,
    }
}

impl Window {
    pub fn set_exclusive_zone(&mut self, zone: i32) {
        if let ShellSurface::Layer(layer) = &self.role {
            layer.set_exclusive_zone(zone);
            self.stage();
        }
    }

    pub fn set_layer(&mut self, kind: LayerKind) {
        if let ShellSurface::Layer(layer) = &self.role {
            layer.set_layer(layer_of(kind));
            self.stage();
        }
    }

    pub fn set_layer_anchor(&mut self, anchor: Edges) {
        if let ShellSurface::Layer(layer) = &self.role {
            layer.set_anchor(anchor_of(anchor));
            self.stage();
        }
    }

    pub fn set_layer_margin(&mut self, top: i32) {
        if let ShellSurface::Layer(layer) = &self.role {
            layer.set_margin(top, 0, 0, 0);
            self.stage();
        }
    }

    pub fn set_layer_size(&mut self, size: (u32, u32)) -> Result<(), WindowError> {
        let ShellSurface::Layer(layer) = &self.role else {
            return Err(WindowError::Missing("a layer surface"));
        };
        layer.set_size(size.0, size.1);
        if !self.visible {
            return Ok(());
        }
        self.state.configured = false;
        self.surface.commit();
        while !self.state.configured {
            self.queue.blocking_dispatch(&mut self.state)?;
        }
        self.stage();
        Ok(())
    }

    pub fn set_input_region(&mut self, region: Option<&[Rect]>) {
        let Some(rects) = region else {
            self.surface.set_input_region(None);
            self.stage();
            return;
        };
        let Some(compositor) = &self.state.compositor else {
            return;
        };
        let wayland_region = compositor.create_region(&self.queue.handle(), ());
        let offset = self.content_offset();
        for rect in rects {
            let left = (rect.x + offset.x).floor() as i32;
            let top = (rect.y + offset.y).floor() as i32;
            wayland_region.add(
                left,
                top,
                (rect.x + offset.x + rect.width).ceil() as i32 - left,
                (rect.y + offset.y + rect.height).ceil() as i32 - top,
            );
        }
        self.surface.set_input_region(Some(&wayland_region));
        wayland_region.destroy();
        self.stage();
    }

    pub fn set_keyboard_focus(&mut self, focus: KeyboardFocus) -> Result<(), WindowError> {
        let ShellSurface::Layer(layer) = &self.role else {
            return Err(WindowError::Missing("a layer surface"));
        };
        layer.set_keyboard_interactivity(keyboard_interactivity(focus));
        self.stage();
        Ok(())
    }
}

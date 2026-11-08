use std::fs;
use std::path::PathBuf;

use smithay::desktop::{LayerMap, LayerSurface, WindowSurfaceType, layer_map_for_output};
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::utils::{Logical, Point, Rectangle};

use crate::recall::{LatestWriter, clamp_into};
use crate::state::{Compositor, State};

const SNAP_DISTANCE: i32 = 12;

type Spot = Point<i32, Logical>;

#[derive(Debug, Clone, PartialEq)]
struct Place {
    namespace: String,
    display: String,
    at: Point<i32, Logical>,
}

#[derive(Default)]
pub(crate) struct LayerPlaces {
    writer: Option<LatestWriter>,
    places: Vec<Place>,
}

struct Held {
    screen: usize,
    namespace: String,
    display: String,
    home: Rectangle<i32, Logical>,
    at: Point<i32, Logical>,
}

fn parse(text: &str) -> Vec<Place> {
    text.lines()
        .filter_map(|line| {
            let mut fields = line.split('\t');
            let namespace = fields.next()?.to_owned();
            let display = fields.next()?.to_owned();
            let x = fields.next()?.parse().ok()?;
            let y = fields.next()?.parse().ok()?;
            (fields.next().is_none() && !namespace.is_empty() && !display.is_empty()).then(|| {
                Place {
                    namespace,
                    display,
                    at: (x, y).into(),
                }
            })
        })
        .collect()
}

fn format(places: &[Place]) -> String {
    places
        .iter()
        .map(|place| {
            format!(
                "{}\t{}\t{}\t{}\n",
                place.namespace, place.display, place.at.x, place.at.y
            )
        })
        .collect()
}

impl LayerPlaces {
    fn find(&self, namespace: &str, display: &str) -> Option<Point<i32, Logical>> {
        self.places
            .iter()
            .find(|place| place.namespace == namespace && place.display == display)
            .map(|place| place.at)
    }

    fn set(&mut self, namespace: &str, display: &str, at: Option<Point<i32, Logical>>) {
        self.places
            .retain(|place| place.namespace != namespace || place.display != display);
        if let Some(at) = at {
            self.places.push(Place {
                namespace: namespace.to_owned(),
                display: display.to_owned(),
                at,
            });
        }
    }
}

impl State {
    fn screen_bounds(&self, screen: usize) -> Rectangle<i32, Logical> {
        Rectangle::new((0, 0).into(), self.screens[screen].rect().size)
    }

    pub(crate) fn layer_location(
        &self,
        screen: usize,
        map: &LayerMap,
        layer: &LayerSurface,
    ) -> Option<Point<i32, Logical>> {
        let geometry = map.layer_geometry(layer)?;
        let namespace = layer.namespace();
        if !self
            .layer_places
            .places
            .iter()
            .any(|place| place.namespace == namespace)
        {
            return Some(geometry.loc);
        }
        let Some(at) = self
            .layer_places
            .find(namespace, &self.screen_identity(screen))
        else {
            return Some(geometry.loc);
        };
        Some(
            clamp_into(
                Rectangle::new(at, geometry.size),
                self.screen_bounds(screen),
            )
            .loc,
        )
    }

    fn held_layer(&self, surface: &WlSurface) -> Option<Held> {
        let screen = self.layer_screen(surface)?;
        let map = layer_map_for_output(&self.screens[screen].output);
        let layer = map.layer_for_surface(surface, WindowSurfaceType::TOPLEVEL)?;
        Some(Held {
            screen,
            namespace: layer.namespace().to_owned(),
            display: self.screen_identity(screen),
            home: map.layer_geometry(layer)?,
            at: self.layer_location(screen, &map, layer)?,
        })
    }

    pub(crate) fn begin_layer_move(&self, surface: &WlSurface) -> Option<(Spot, Option<Spot>)> {
        let held = self.held_layer(surface)?;
        let remembered = self.layer_places.find(&held.namespace, &held.display);
        Some((held.at, remembered))
    }

    pub(crate) fn move_layer_to(&mut self, surface: &WlSurface, at: Point<i32, Logical>) {
        let Some(held) = self.held_layer(surface) else {
            return;
        };
        let at = clamp_into(
            Rectangle::new(at, held.home.size),
            self.screen_bounds(held.screen),
        )
        .loc;
        let away = (at.x - held.home.loc.x)
            .abs()
            .max((at.y - held.home.loc.y).abs())
            > SNAP_DISTANCE;
        self.layer_places
            .set(&held.namespace, &held.display, away.then_some(at));
        self.needs_redraw = true;
    }

    pub(crate) fn put_layer_back(&mut self, surface: &WlSurface, at: Option<Point<i32, Logical>>) {
        if let Some(held) = self.held_layer(surface) {
            self.layer_places.set(&held.namespace, &held.display, at);
            self.needs_redraw = true;
        }
    }

    pub(crate) fn remember_layer_places(&self) {
        if let Some(writer) = &self.layer_places.writer {
            writer.write(format(&self.layer_places.places));
        }
    }
}

impl Compositor {
    pub fn remember_layer_places_in(&mut self, path: PathBuf) {
        let places = fs::read_to_string(&path)
            .map(|text| parse(&text))
            .unwrap_or_default();
        let writer = LatestWriter::start(path, "layer places")
            .inspect_err(|error| tracing::warn!("layer places are not remembered: {error}"))
            .ok();
        self.state.layer_places = LayerPlaces { writer, places };
    }
}

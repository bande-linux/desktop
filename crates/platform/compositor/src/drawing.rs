use std::os::fd::OwnedFd;

use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{Interest, Mode, PostAction};

use crate::element::ScreenElement;
use crate::state::Compositor;

#[derive(Default)]
pub(crate) struct InFlight {
    frames: Vec<(u64, Vec<ScreenElement>)>,
    watched: u64,
}

impl InFlight {
    fn drawn(&mut self, completed: u64) {
        self.frames.retain(|(serial, _)| *serial > completed);
    }
}

impl Compositor {
    pub fn keep_until_drawn(
        &mut self,
        elements: Vec<ScreenElement>,
        (submitted, completed): (u64, u64),
        fence: Option<OwnedFd>,
    ) {
        let in_flight = &mut self.state.in_flight;
        in_flight.drawn(completed);
        if submitted <= completed {
            return;
        }
        in_flight.frames.push((submitted, elements));
        let Some(fence) = fence.filter(|_| submitted > in_flight.watched) else {
            return;
        };
        let watched = self.state.blockers.insert_source(
            Generic::new(fence, Interest::READ, Mode::Level),
            move |_, _, state| {
                state.in_flight.drawn(submitted);
                Ok(PostAction::Remove)
            },
        );
        match watched {
            Ok(_) => self.state.in_flight.watched = submitted,
            Err(error) => tracing::warn!("cannot wait for the GPU to finish a frame: {error}"),
        }
    }
}

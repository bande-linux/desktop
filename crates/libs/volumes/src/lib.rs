mod eject;
mod mounts;

pub use eject::{EjectFailure, eject};
pub use mounts::{media_roots, mounted_volumes};

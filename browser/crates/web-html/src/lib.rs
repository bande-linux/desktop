#![forbid(unsafe_code)]

mod sink;

pub use sink::{Attributes, QuirksMode, TreeSink};
pub use web_atoms::{Atom, Namespace, QualName};

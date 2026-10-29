mod calc;
mod index;
mod matcher;
mod usage;

pub use calc::{calculate, format_number};
pub use index::{FileHit, FileIndex, FileResults, MAX_HITS};
pub use matcher::{EXACT, FUZZY, INITIALS, PREFIX, Query, SUBSTRING, WORD, WORDS, fold};
pub use usage::{MAX_BOOST, Usage, usage_path};

mod calc;
mod index;
mod matcher;
mod recents;
mod usage;

pub use calc::{Calculation, calculate, format_number};
pub use index::{FileHit, FileIndex, FileResults, MAX_HITS};
pub use matcher::{EXACT, FUZZY, INITIALS, PREFIX, Query, SUBSTRING, WORD, WORDS, fold};
pub use recents::{FILE_KEY, recent_files, recently_used, recently_used_path};
pub use usage::{MAX_BOOST, Usage, usage_path};

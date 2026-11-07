mod calc;
mod index;
mod matcher;
mod recents;
mod service;
mod tree;
mod usage;

pub use calc::{Calculation, calculate, format_number};
pub use index::{CACHE_TAG, FileHit, FileIndex, FileResults, MAX_ENTRIES, MAX_HITS, skipped_name};
pub use matcher::{EXACT, FUZZY, INITIALS, PREFIX, Query, SUBSTRING, WORD, WORDS, fold};
pub use recents::{
    FILE_KEY, RecentlyUsed, Recents, is_excluded, recent_files, recently_used, recently_used_path,
};
pub use service::{
    BUS_NAME, Donation, Found, IndexProxyBlocking, OBJECT_PATH, Policy, SearchService,
    follow_policy,
};
pub use tree::{ROOT, Restored, Stamp, Tree};
pub use usage::{MAX_BOOST, Usage, usage_path};

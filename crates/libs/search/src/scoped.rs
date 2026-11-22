use std::fs::File;
use std::io::{self, Read};
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};

use crate::{FUZZY, FileIndex, FileResults, SearchService, Usage, recent_files, recently_used};

const MAX_XBEL_BYTES: u64 = 8 << 20;

/// The files in `recently-used.xbel` at `xbel`, most used first by `usage`, without those in
/// `excluded` folders, at most `limit` of them.
pub fn load_recent_files(
    xbel: &Path,
    usage: PathBuf,
    excluded: &[PathBuf],
    limit: usize,
) -> Vec<PathBuf> {
    let mut bytes = Vec::new();
    match File::open(xbel).and_then(|file| file.take(MAX_XBEL_BYTES).read_to_end(&mut bytes)) {
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!("cannot read {}: {error}", xbel.display()),
    }
    let xbel = String::from_utf8_lossy(&bytes);
    recent_files(&recently_used(&xbel), &Usage::load(usage), excluded, limit)
}

enum Searcher {
    Home(SearchService),
    Elsewhere(FileIndex),
}

/// A file search within a folder: through the search service inside the indexed part of the
/// home folder, and with an index of its own anywhere else.
pub struct ScopedSearch {
    root: PathBuf,
    searcher: Searcher,
    generation: u64,
}

fn covered(root: &Path, folder: &Path) -> bool {
    folder.strip_prefix(root).is_ok_and(|inside| {
        inside
            .components()
            .all(|part| !bande_fs::is_hidden(&part.as_os_str().to_string_lossy()))
    })
}

impl ScopedSearch {
    fn start(home: &Path, scope: &Path, excluded: &[PathBuf]) -> io::Result<Self> {
        let (root, searcher) = match covered(home, scope) {
            true => (
                home.to_path_buf(),
                Searcher::Home(SearchService::connect()?),
            ),
            false => (
                scope.to_path_buf(),
                Searcher::Elsewhere(FileIndex::spawn(scope.to_path_buf(), excluded.to_vec())?),
            ),
        };
        Ok(Self {
            root,
            searcher,
            generation: 0,
        })
    }

    /// Asks for up to `limit` files within `scope` that match `text`, first starting a search in
    /// `search` that covers `scope` unless the one there already does.
    pub fn query(
        search: &mut Option<Self>,
        home: &Path,
        scope: &Path,
        text: &str,
        excluded: &[PathBuf],
        limit: usize,
    ) -> io::Result<()> {
        let search = match search {
            Some(search) if covered(&search.root, scope) => search,
            _ => search.insert(Self::start(home, scope, excluded)?),
        };
        search.generation += 1;
        match &search.searcher {
            Searcher::Home(service) => service.query_within(search.generation, text, scope, limit),
            Searcher::Elsewhere(index) => index.query_within(search.generation, text, scope, limit),
        }
        Ok(())
    }

    pub fn exclude(&self, excluded: &[PathBuf]) {
        if let Searcher::Elsewhere(index) = &self.searcher {
            index.exclude(excluded.to_vec());
        }
    }

    pub fn fd(&self) -> BorrowedFd<'_> {
        match &self.searcher {
            Searcher::Home(service) => service.fd(),
            Searcher::Elsewhere(index) => index.fd(),
        }
    }

    /// The files found for the latest query, once they arrive.
    pub fn take(&mut self) -> Option<Vec<PathBuf>> {
        let results: FileResults = match &mut self.searcher {
            Searcher::Home(service) => service.take(),
            Searcher::Elsewhere(index) => index.take(),
        }?;
        (results.generation == self.generation).then(|| {
            results
                .hits
                .into_iter()
                .filter(|hit| hit.score > FUZZY)
                .map(|hit| hit.path)
                .collect()
        })
    }
}

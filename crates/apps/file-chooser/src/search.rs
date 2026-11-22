use std::fs::File;
use std::io::{self, Read};
use std::os::fd::BorrowedFd;
use std::path::{Path, PathBuf};

use bande_fs::is_hidden;
use bande_search::{
    FUZZY, FileIndex, FileResults, SearchService, Usage, recent_files, recently_used,
    recently_used_path, usage_path,
};

const MAX_RECENTS: usize = 200;
const MAX_FOUND: usize = 500;
const MAX_XBEL_BYTES: u64 = 8 << 20;

fn read_xbel(path: &Path) -> String {
    let mut bytes = Vec::new();
    match File::open(path).and_then(|file| file.take(MAX_XBEL_BYTES).read_to_end(&mut bytes)) {
        Ok(_) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            tracing::warn!("cannot read {}: {error}", path.display());
            String::new()
        }
    }
}

pub fn recent_paths(excluded: &[PathBuf]) -> Vec<PathBuf> {
    let xbel = recently_used_path()
        .map(|path| read_xbel(&path))
        .unwrap_or_default();
    let usage = usage_path().map(Usage::load).unwrap_or_default();
    recent_files(&recently_used(&xbel), &usage, excluded, MAX_RECENTS)
}

enum Searcher {
    Home(SearchService),
    Elsewhere(FileIndex),
}

pub struct Search {
    root: PathBuf,
    searcher: Searcher,
    generation: u64,
}

fn covered(root: &Path, folder: &Path) -> bool {
    folder.strip_prefix(root).is_ok_and(|inside| {
        inside
            .components()
            .all(|part| !is_hidden(&part.as_os_str().to_string_lossy()))
    })
}

impl Search {
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

    pub fn query(
        search: &mut Option<Self>,
        home: &Path,
        scope: &Path,
        text: &str,
        excluded: &[PathBuf],
    ) {
        if !search
            .as_ref()
            .is_some_and(|search| covered(&search.root, scope))
        {
            match Self::start(home, scope, excluded) {
                Ok(started) => *search = Some(started),
                Err(error) => {
                    tracing::warn!("cannot search {}: {error}", scope.display());
                    return;
                }
            }
        }
        let Some(search) = search else {
            return;
        };
        search.generation += 1;
        match &search.searcher {
            Searcher::Home(service) => {
                service.query_within(search.generation, text, scope, MAX_FOUND);
            }
            Searcher::Elsewhere(index) => {
                index.query_within(search.generation, text, scope, MAX_FOUND);
            }
        }
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

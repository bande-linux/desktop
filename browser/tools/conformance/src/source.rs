use std::path::{Path, PathBuf};
use std::process::Command;

pub struct Source {
    pub name: &'static str,
    pub url: &'static str,
    pub commit: &'static str,
    pub paths: &'static [&'static str],
}

pub const HTML5LIB: Source = Source {
    name: "html5lib-tests",
    url: "https://github.com/html5lib/html5lib-tests",
    commit: "c777c408b61078ea2eb4acefc2535f54dbc8b28a",
    paths: &[],
};

pub const CSS_PARSING: Source = Source {
    name: "css-parsing-tests",
    url: "https://github.com/SimonSapin/css-parsing-tests",
    commit: "203ce36bffd617db7f118c551e32794561fb273d",
    paths: &[],
};

pub const WPT: Source = Source {
    name: "wpt",
    url: "https://github.com/web-platform-tests/wpt",
    commit: "5197f4e83e1e5a0e02c85f8600c11ae9c0433769",
    paths: &[
        "/html/syntax/parsing/resources/",
        "/css/CSS2/",
        "/css/css-cascade/",
        "/css/css-syntax/",
        "/css/selectors/",
        "/css/reference/",
        "/css/support/",
        "/fonts/",
        "/images/",
    ],
};

pub fn cache_root() -> PathBuf {
    let env = |name| std::env::var_os(name).map(PathBuf::from);
    env("WEB_SUITES")
        .or_else(|| env("XDG_CACHE_HOME").map(|dir| dir.join("bande-web/suites")))
        .or_else(|| env("HOME").map(|dir| dir.join(".cache/bande-web/suites")))
        .unwrap_or_else(|| PathBuf::from("target/suites"))
}

impl Source {
    pub fn checkout(&self) -> Result<PathBuf, String> {
        let dir = cache_root().join(self.name);
        let stamp_path = dir.join(".git/bande-pin");
        let stamp = format!("{} {}\n", self.commit, self.paths.join(" "));
        if std::fs::read_to_string(&stamp_path).is_ok_and(|found| found == stamp) {
            return Ok(dir);
        }
        eprintln!("fetching {} at {}", self.name, self.commit);
        std::fs::create_dir_all(&dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        git(&dir, &["init", "--quiet"])?;
        if self.paths.is_empty() {
            git(&dir, &["sparse-checkout", "disable"])?;
        } else {
            let mut args = vec!["sparse-checkout", "set", "--no-cone"];
            args.extend(self.paths);
            git(&dir, &args)?;
        }
        git(
            &dir,
            &[
                "fetch",
                "--quiet",
                "--depth=1",
                "--filter=blob:none",
                self.url,
                self.commit,
            ],
        )?;
        git(&dir, &["checkout", "--quiet", "--force", "FETCH_HEAD"])?;
        std::fs::write(&stamp_path, stamp)
            .map_err(|error| format!("{}: {error}", stamp_path.display()))?;
        Ok(dir)
    }
}

fn git(dir: &Path, args: &[&str]) -> Result<(), String> {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .map_err(|error| format!("git: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "git {} failed in {}",
            args.join(" "),
            dir.display()
        ))
    }
}

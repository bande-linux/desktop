use std::collections::HashSet;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::naming::split_extension;
use crate::ops::Change;

const COUNTER_DIGITS: usize = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Numbering {
    Index,
    Counter,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameRule {
    Replace {
        find: String,
        replace: String,
    },
    Add {
        text: String,
        after: bool,
    },
    Format {
        name: String,
        numbering: Numbering,
        start: u64,
        after: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchProblem {
    Empty,
    ContainsSlash,
    Reserved,
    Twice(String),
    Exists(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BatchItem {
    pub path: PathBuf,
    pub folder: bool,
}

fn name_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn number(numbering: Numbering, value: u64) -> String {
    match numbering {
        Numbering::Index => value.to_string(),
        Numbering::Counter => format!("{value:0COUNTER_DIGITS$}"),
    }
}

pub fn new_name(name: &str, folder: bool, rule: &RenameRule, position: usize) -> String {
    let (stem, extension) = split_extension(name, folder);
    let stem = match rule {
        RenameRule::Replace { find, replace } if !find.is_empty() => stem.replace(find, replace),
        RenameRule::Replace { .. } => stem.to_owned(),
        RenameRule::Add { text, after: true } => format!("{stem}{text}"),
        RenameRule::Add { text, after: false } => format!("{text}{stem}"),
        RenameRule::Format {
            name,
            numbering,
            start,
            after,
        } => {
            let number = number(*numbering, start.saturating_add(position as u64));
            match (name.is_empty(), after) {
                (true, _) => number,
                (false, true) => format!("{name} {number}"),
                (false, false) => format!("{number} {name}"),
            }
        }
    };
    format!("{stem}{extension}")
}

pub fn plan_renames(
    items: &[BatchItem],
    rule: &RenameRule,
) -> Result<Vec<(PathBuf, PathBuf)>, BatchProblem> {
    let sources = items
        .iter()
        .map(|item| item.path.as_path())
        .collect::<HashSet<_>>();
    let mut taken = HashSet::new();
    let mut plan = Vec::new();
    for (position, item) in items.iter().enumerate() {
        let old = name_of(&item.path);
        let name = new_name(&old, item.folder, rule, position);
        if name.is_empty() || (name.starts_with('.') && !old.starts_with('.')) {
            return Err(BatchProblem::Empty);
        }
        if name.contains('/') {
            return Err(BatchProblem::ContainsSlash);
        }
        if name == "." || name == ".." {
            return Err(BatchProblem::Reserved);
        }
        let target = item.path.with_file_name(&name);
        if !taken.insert(target.clone()) {
            return Err(BatchProblem::Twice(name));
        }
        if target == item.path {
            continue;
        }
        if !sources.contains(target.as_path()) && target.symlink_metadata().is_ok() {
            return Err(BatchProblem::Exists(name));
        }
        plan.push((item.path.clone(), target));
    }
    Ok(plan)
}

fn moved(from: &Path, to: &Path) -> io::Result<Change> {
    fs::rename(from, to)?;
    Ok(Change::Moved {
        from: from.to_path_buf(),
        to: to.to_path_buf(),
    })
}

fn staging(path: &Path, index: usize) -> PathBuf {
    path.with_file_name(format!(
        ".{}.renaming-{}-{index}",
        name_of(path),
        std::process::id()
    ))
}

pub fn rename_all(plan: &[(PathBuf, PathBuf)]) -> (Vec<Change>, Option<io::Error>) {
    let sources = plan
        .iter()
        .map(|(from, _)| from.as_path())
        .collect::<HashSet<_>>();
    let chained = plan.iter().any(|(_, to)| sources.contains(to.as_path()));
    let mut changes = Vec::new();
    let steps = match chained {
        false => plan.to_vec(),
        true => {
            let staged = plan
                .iter()
                .enumerate()
                .map(|(index, (from, to))| (from.clone(), staging(from, index), to.clone()))
                .collect::<Vec<_>>();
            for (from, stage, _) in &staged {
                match moved(from, stage) {
                    Ok(change) => changes.push(change),
                    Err(error) => return (changes, Some(error)),
                }
            }
            staged
                .into_iter()
                .map(|(_, stage, to)| (stage, to))
                .collect()
        }
    };
    for (from, to) in steps {
        match moved(&from, &to) {
            Ok(change) => changes.push(change),
            Err(error) => return (changes, Some(error)),
        }
    }
    (changes, None)
}

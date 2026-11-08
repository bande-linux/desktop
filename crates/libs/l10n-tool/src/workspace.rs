use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::str::FromStr;

use bande_l10n_source::key;
use bande_l10n_source::pattern::placeholders;
use bande_l10n_source::syntax::{Invocation, invocations};
use proc_macro2::TokenStream;
use serde_json::Value;

const RUNTIME: &str = "bande-l10n";
const DEVELOPMENT_ASSET: &str = "target/release/l10n/development";

#[derive(Debug, Clone)]
pub struct Key {
    pub context: Option<String>,
    pub english: String,
    pub plural: Option<String>,
}

impl Key {
    pub fn id(&self) -> String {
        key(self.context.as_deref(), &self.english)
    }

    pub fn placeholders(&self) -> Vec<&str> {
        [Some(self.english.as_str()), self.plural.as_deref()]
            .into_iter()
            .flatten()
            .flat_map(|text| placeholders(text).unwrap_or_default())
            .collect()
    }
}

pub struct Crate {
    pub name: String,
    pub folder: PathBuf,
    pub keys: Vec<Key>,
}

impl Crate {
    pub fn locales(&self) -> PathBuf {
        self.folder.join("locales")
    }
}

pub struct Program {
    pub name: String,
    closure: BTreeSet<String>,
}

pub struct Workspace {
    pub development: String,
    pub crates: Vec<Crate>,
    pub programs: Vec<Program>,
    pub problems: Vec<String>,
}

impl Workspace {
    pub fn load() -> Result<Self, Vec<String>> {
        let output = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned()))
            .args(["metadata", "--format-version", "1", "--locked"])
            .output()
            .map_err(|error| vec![format!("cannot run cargo metadata: {error}")])?;
        if !output.status.success() {
            return Err(vec![String::from_utf8_lossy(&output.stderr).into_owned()]);
        }
        let metadata: Value = serde_json::from_slice(&output.stdout)
            .map_err(|error| vec![format!("cargo metadata: {error}")])?;
        let development = metadata["metadata"]["l10n"]["development-language"]
            .as_str()
            .ok_or_else(|| {
                vec![
                    "the workspace declares no [workspace.metadata.l10n] development-language"
                        .to_owned(),
                ]
            })?
            .to_owned();
        let packages = metadata["packages"].as_array().cloned().unwrap_or_default();
        let by_id = packages
            .iter()
            .filter_map(|package| Some((package["id"].as_str()?.to_owned(), package)))
            .collect::<BTreeMap<_, _>>();
        let mut problems = Vec::new();
        let mut crates = packages
            .iter()
            .filter(|package| {
                package["dependencies"]
                    .as_array()
                    .is_some_and(|dependencies| {
                        dependencies
                            .iter()
                            .any(|dependency| dependency["name"] == RUNTIME)
                    })
            })
            .filter_map(|package| {
                let name = package["name"].as_str()?.to_owned();
                let folder = Path::new(package["manifest_path"].as_str()?)
                    .parent()?
                    .to_owned();
                let keys = keys_of(&folder.join("src"), &mut problems);
                Some(Crate { name, folder, keys })
            })
            .collect::<Vec<_>>();
        crates.sort_by(|left, right| left.name.cmp(&right.name));
        let dependencies = metadata["resolve"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|node| {
                let normal = node["deps"]
                    .as_array()?
                    .iter()
                    .filter(|dependency| {
                        dependency["dep_kinds"]
                            .as_array()
                            .is_some_and(|kinds| kinds.iter().any(|kind| kind["kind"].is_null()))
                    })
                    .filter_map(|dependency| dependency["pkg"].as_str().map(str::to_owned))
                    .collect::<Vec<_>>();
                Some((node["id"].as_str()?.to_owned(), normal))
            })
            .collect::<BTreeMap<_, _>>();
        let members = metadata["workspace_members"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .filter_map(|id| by_id.get(id).map(|package| (id, *package)))
            .collect::<Vec<_>>();
        let assets = members
            .iter()
            .flat_map(|(_, package)| {
                let owner = package["name"].as_str().unwrap_or_default().to_owned();
                package["metadata"]["deb"]["assets"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(move |asset| {
                        Some((owner.clone(), asset.get(0)?.as_str()?.to_owned()))
                    })
            })
            .collect::<Vec<_>>();
        let shipped = assets
            .iter()
            .filter_map(|(owner, source)| {
                Some((
                    source.strip_prefix("target/release/")?.to_owned(),
                    owner.clone(),
                ))
            })
            .collect::<BTreeMap<_, _>>();
        let mut programs = Vec::new();
        for (id, package) in &members {
            for target in package["targets"].as_array().into_iter().flatten() {
                let is_binary = target["kind"]
                    .as_array()
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind == "bin"));
                let Some(name) = target["name"].as_str().filter(|_| is_binary) else {
                    continue;
                };
                let owner = shipped.get(name);
                if !assets.is_empty() && owner.is_none() {
                    continue;
                }
                let mut closure = BTreeSet::new();
                let mut pending = vec![id.to_string()];
                while let Some(next) = pending.pop() {
                    if let Some(package) = by_id.get(&next)
                        && closure.insert(package["name"].as_str().unwrap_or_default().to_owned())
                    {
                        pending.extend(dependencies.get(&next).into_iter().flatten().cloned());
                    }
                }
                let program = Program {
                    name: name.to_owned(),
                    closure,
                };
                let has_keys = crates
                    .iter()
                    .any(|krate| !krate.keys.is_empty() && program.closure.contains(&krate.name));
                if let Some(owner) = owner.filter(|_| has_keys) {
                    let asset = format!("{DEVELOPMENT_ASSET}/{name}/*");
                    if !assets
                        .iter()
                        .any(|(holder, source)| holder == owner && *source == asset)
                    {
                        problems.push(format!("{owner}: ship {name}'s English with the asset [\"{asset}\", \"usr/share/bande/l10n/{name}/\", \"644\"]"));
                    }
                }
                programs.push(program);
            }
        }
        Ok(Self {
            development,
            crates,
            programs,
            problems,
        })
    }

    pub fn languages(&self) -> Vec<String> {
        let mut languages = self
            .crates
            .iter()
            .flat_map(|krate| {
                std::fs::read_dir(krate.locales())
                    .into_iter()
                    .flatten()
                    .flatten()
            })
            .filter_map(|entry| {
                entry
                    .file_name()
                    .into_string()
                    .ok()?
                    .strip_suffix(".po")
                    .map(str::to_owned)
            })
            .collect::<Vec<_>>();
        languages.sort();
        languages.dedup();
        languages
    }

    pub fn crates_of(&self, program: &Program) -> Vec<&Crate> {
        self.crates
            .iter()
            .filter(|krate| program.closure.contains(&krate.name))
            .collect()
    }

    pub fn programs_with(&self, krate: &str) -> Vec<String> {
        self.programs
            .iter()
            .filter(|program| program.closure.contains(krate))
            .map(|program| program.name.clone())
            .collect()
    }
}

fn keys_of(folder: &Path, problems: &mut Vec<String>) -> Vec<Key> {
    let mut files = Vec::new();
    let mut pending = vec![folder.to_owned()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(&next).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    let mut keys = Vec::<Key>::new();
    for file in files {
        let source = match std::fs::read_to_string(&file) {
            Ok(source) => source,
            Err(error) => {
                problems.push(format!("{}: {error}", file.display()));
                continue;
            }
        };
        let tokens = match TokenStream::from_str(&source) {
            Ok(tokens) => tokens,
            Err(error) => {
                problems.push(format!("{}: {error}", file.display()));
                continue;
            }
        };
        let mut found = Vec::new();
        let mut errors = Vec::new();
        invocations(tokens, &mut found, &mut errors);
        problems.extend(
            errors
                .into_iter()
                .map(|error| format!("{}: {error}", file.display())),
        );
        for Invocation {
            context,
            english,
            plural,
            ..
        } in found
        {
            let found = Key {
                context,
                english,
                plural,
            };
            match keys.iter().find(|known| known.id() == found.id()) {
                Some(known) if known.plural != found.plural => problems.push(format!(
                    "{}: “{}” is used with two different plurals; give one a context",
                    file.display(),
                    found.english
                )),
                Some(_) => {}
                None => keys.push(found),
            }
        }
    }
    keys
}

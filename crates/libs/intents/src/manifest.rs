use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Definition, Label, Property, ThingDefinition, domain};

/// Where packages install manifests.
pub const MANIFESTS: &str = "/usr/share/bande/intents";
const EXTENSION: &str = "json";

/// What one program serves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// The bus name it serves on.
    pub bus: String,
    pub program: PathBuf,
    #[serde(default)]
    pub things: BTreeMap<String, ThingDefinition>,
    #[serde(default)]
    pub intents: BTreeMap<String, Definition>,
}

impl Manifest {
    /// The catalogs of the program, for its [`Label`]s.
    pub fn catalogs(&self) -> bande_l10n::Program {
        let name = self.program.file_name().unwrap_or_default();
        bande_l10n::Program::named(&name.to_string_lossy())
    }

    /// `label` in the person's language.
    pub fn label(&self, label: &Label) -> String {
        self.catalogs().pattern(label.context.as_deref(), &label.en)
    }

    pub(crate) fn write(&self, folder: &Path) -> io::Result<()> {
        let mut contents = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        contents.push('\n');
        std::fs::create_dir_all(folder)?;
        std::fs::write(folder.join(format!("{}.{EXTENSION}", self.bus)), contents)
    }
}

/// The manifests of the programs that are installed, each id and thing type served by one.
#[derive(Debug, Clone, Default)]
pub struct Manifests(Vec<Manifest>);

impl Manifests {
    /// Reads the manifests in `folder`, in the order of their file names; what a second one
    /// serves again, and a file that does not parse, is logged and left out.
    pub fn read(folder: &Path) -> Self {
        let mut paths: Vec<PathBuf> = match std::fs::read_dir(folder) {
            Ok(entries) => entries
                .filter_map(|entry| Some(entry.ok()?.path()))
                .filter(|path| path.extension().is_some_and(|it| it == EXTENSION))
                .collect(),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
            Err(error) => {
                tracing::warn!("cannot read {}: {error}", folder.display());
                Vec::new()
            }
        };
        paths.sort();
        let mut manifests = Self::default();
        for path in paths {
            let read = std::fs::read(&path)
                .map_err(serde_json::Error::io)
                .and_then(|contents| serde_json::from_slice::<Manifest>(&contents));
            match read {
                Ok(manifest) => manifests.add(manifest, &path),
                Err(error) => tracing::warn!("cannot read {}: {error}", path.display()),
            }
        }
        manifests
    }

    fn add(&mut self, mut manifest: Manifest, path: &Path) {
        manifest.intents.retain(|id, _| {
            let taken = self.intent(id).is_some();
            if taken {
                tracing::warn!("{} serves {id}, which another does", path.display());
            }
            !taken
        });
        manifest
            .things
            .retain(|thing, _| self.thing(thing).is_none());
        self.0.push(manifest);
    }

    /// The manifests installed on this computer.
    pub fn installed() -> Self {
        Self::read(Path::new(MANIFESTS))
    }

    pub fn iter(&self) -> impl Iterator<Item = &Manifest> {
        self.0.iter()
    }

    pub(crate) fn bus_names(&self) -> impl Iterator<Item = &str> {
        self.0.iter().map(|manifest| manifest.bus.as_str())
    }

    pub fn intent(&self, id: &str) -> Option<(&Manifest, &Definition)> {
        self.0
            .iter()
            .find_map(|manifest| Some((manifest, manifest.intents.get(id)?)))
    }

    pub fn thing(&self, kind: &str) -> Option<(&Manifest, &BTreeMap<String, Property>)> {
        self.0
            .iter()
            .find_map(|manifest| Some((manifest, &manifest.things.get(kind)?.properties)))
    }

    /// The manifest of the program whose receipts are named with `of`'s domain.
    pub fn serving(&self, of: &str) -> Option<&Manifest> {
        let wanted = domain(of);
        self.0
            .iter()
            .find(|manifest| manifest.intents.keys().any(|id| domain(id) == wanted))
    }
}

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use bande_l10n_source::catalog::Catalog;

use crate::workspace::Workspace;

const MAINTAINER: &str = "Mikael Hug <mikaelhug@gmail.com>";

pub struct Pack<'a> {
    pub catalogs: &'a Path,
    pub out: &'a Path,
    pub version: &'a str,
    pub package: &'a str,
    pub copyright: &'a Path,
}

pub fn build(workspace: &Workspace, pack: &Pack<'_>) -> Result<(), String> {
    let owned = workspace
        .programs
        .iter()
        .filter(|program| program.package.as_deref() == Some(pack.package))
        .map(|program| program.name.clone())
        .collect::<Vec<_>>();
    let mut languages = read_names(&pack.catalogs.join("packs"))?;
    languages.sort();
    fs::create_dir_all(pack.out).map_err(|error| format!("{}: {error}", pack.out.display()))?;
    for language in languages {
        let folder = pack.catalogs.join("packs").join(&language);
        let programs = read_names(&folder)?
            .into_iter()
            .filter(|program| owned.is_empty() || owned.contains(program))
            .collect::<Vec<_>>();
        if programs.is_empty() {
            continue;
        }
        build_one(pack, &language, &folder, &programs, !owned.is_empty())?;
    }
    Ok(())
}

fn build_one(
    pack: &Pack<'_>,
    language: &str,
    folder: &Path,
    programs: &[String],
    depends: bool,
) -> Result<(), String> {
    let name = format!("{}-l10n-{}", pack.package, language.to_lowercase());
    let scratch = std::env::temp_dir().join(format!("bande-l10n-{}-{name}", std::process::id()));
    let root = scratch.join(&name);
    let team = programs
        .iter()
        .find_map(|program| {
            let data = fs::read(folder.join(program).join(format!("{language}.mo"))).ok()?;
            Catalog::parse(data).ok()?.header("Language-Team")
        })
        .ok_or_else(|| format!("no Language-Team in the {language} catalogs"))?;
    let catalogs = root.join("usr/share/bande/l10n");
    let docs = root.join("usr/share/doc").join(&name);
    for program in programs {
        let target = catalogs.join(program);
        let file = format!("{language}.mo");
        fs::create_dir_all(&target)
            .and_then(|()| fs::copy(folder.join(program).join(&file), target.join(&file)))
            .map_err(|error| format!("{}: {error}", target.display()))?;
    }
    let epoch = match std::env::var("SOURCE_DATE_EPOCH") {
        Ok(epoch) => epoch,
        Err(_) => run(Command::new("git").args(["log", "-1", "--format=%ct"]))?,
    };
    let date = run(Command::new("date").args(["-R", "-u", "-d", &format!("@{}", epoch.trim())]))?;
    let changelog = format!(
        "{name} ({}) unstable; urgency=medium\n\n  * Built from the translations at this version.\n\n -- {MAINTAINER}  {}\n",
        pack.version,
        date.trim()
    );
    fs::create_dir_all(&docs)
        .and_then(|()| fs::copy(pack.copyright, docs.join("copyright")).map(drop))
        .and_then(|()| fs::write(docs.join("changelog"), changelog))
        .map_err(|error| format!("{}: {error}", docs.display()))?;
    run(Command::new("gzip")
        .args(["-9n"])
        .arg(docs.join("changelog")))?;
    run(Command::new("chmod").args(["-R", "u=rwX,go=rX"]).arg(&root))?;
    let size = run(Command::new("du").args(["-sk"]).arg(root.join("usr")))?;
    let relation = if depends {
        format!("Depends: {}\n", pack.package)
    } else {
        String::new()
    };
    let description = format!(
        "The {team} text of {}, read by each of its programs when it starts in a {team} session. Without it they show the English they were written in.",
        pack.package
    );
    let control = format!(
        "Package: {name}\nVersion: {}\nArchitecture: all\nMaintainer: {MAINTAINER}\nInstalled-Size: {}\n{relation}Enhances: {}\nSection: localization\nPriority: optional\nMulti-Arch: foreign\nDescription: {team} translations for {}\n{}\n",
        pack.version,
        size.split_whitespace().next().unwrap_or("0"),
        pack.package,
        pack.package,
        wrapped(&description)
    );
    let debian = root.join("DEBIAN");
    fs::create_dir_all(&debian)
        .and_then(|()| fs::write(debian.join("control"), control))
        .map_err(|error| format!("{}: {error}", debian.display()))?;
    let sums = run(Command::new("sh")
        .arg("-c")
        .arg("find usr -type f -print0 | sort -z | xargs -0 md5sum")
        .current_dir(&root))?;
    fs::write(debian.join("md5sums"), sums).map_err(|error| error.to_string())?;
    run(Command::new("chmod").args(["0755"]).arg(&root).arg(&debian))?;
    run(Command::new("chmod")
        .args(["0644"])
        .arg(debian.join("control"))
        .arg(debian.join("md5sums")))?;
    let deb = pack.out.join(format!("{name}_{}_all.deb", pack.version));
    run(Command::new("dpkg-deb")
        .env("SOURCE_DATE_EPOCH", epoch.trim())
        .args(["--root-owner-group", "-Zxz", "--build"])
        .arg(&root)
        .arg(&deb))?;
    let _ = fs::remove_dir_all(&scratch);
    println!("{}", deb.display());
    Ok(())
}

fn wrapped(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(line) if line.len() + 1 + word.len() <= 78 => {
                line.push(' ');
                line.push_str(word);
            }
            _ => lines.push(format!(" {word}")),
        }
    }
    lines.join("\n")
}

fn read_names(folder: &Path) -> Result<Vec<String>, String> {
    let entries = fs::read_dir(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    Ok(entries
        .flatten()
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect())
}

fn run(command: &mut Command) -> Result<String, String> {
    let output = command
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("{command:?}: {error}"))?;
    if !output.status.success() {
        let _ = std::io::stderr().write_all(&output.stderr);
        return Err(format!("{command:?} failed"));
    }
    String::from_utf8(output.stdout).map_err(|error| error.to_string())
}

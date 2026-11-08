mod po;
mod workspace;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use bande_l10n_source::catalog::{self, Entry};
use bande_l10n_source::pattern::placeholders;
use bande_l10n_source::plural::PluralRule;
use bande_l10n_source::{DEVELOPMENT_HEADER, key};

use crate::po::{File, Message, header_field};
use crate::workspace::{Crate, Key, Workspace};

const USAGE: &str =
    "usage: bande-l10n-tool check [complete-languages-file] | update | pack <folder>";

fn main() -> ExitCode {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let result = Workspace::load().and_then(|workspace| {
        match args.iter().map(String::as_str).collect::<Vec<_>>()[..] {
            ["check"] => check(&workspace, &BTreeSet::new()),
            ["check", complete] => std::fs::read_to_string(complete)
                .map_err(|error| vec![format!("cannot read {complete}: {error}")])
                .and_then(|list| {
                    check(
                        &workspace,
                        &list.split_whitespace().map(str::to_owned).collect(),
                    )
                }),
            ["update"] => update(&workspace),
            ["pack", folder] => pack(&workspace, Path::new(folder)),
            _ => Err(vec![USAGE.to_owned()]),
        }
    });
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(problems) => {
            for problem in problems {
                eprintln!("{problem}");
            }
            ExitCode::FAILURE
        }
    }
}

struct Translations {
    file: PathBuf,
    po: File,
}

fn translations(krate: &Crate, language: &str) -> Result<Option<Translations>, String> {
    let file = krate.locales().join(format!("{language}.po"));
    if !file.is_file() {
        return Ok(None);
    }
    let source =
        std::fs::read_to_string(&file).map_err(|error| format!("{}: {error}", file.display()))?;
    let po = po::parse(&source).map_err(|error| format!("{}: {error}", file.display()))?;
    Ok(Some(Translations { file, po }))
}

fn regional(language: &str) -> bool {
    language.contains('-')
}

fn plural_rule(po: &File) -> Result<PluralRule, String> {
    header_field(&po.header, "Plural-Forms").map_or(Ok(PluralRule::default()), PluralRule::parse)
}

fn check(workspace: &Workspace, complete: &BTreeSet<String>) -> Result<(), Vec<String>> {
    let mut problems = [workspace.problems.clone(), workspace.packaging.clone()].concat();
    let mut counts = BTreeMap::<String, (usize, usize)>::new();
    let mut chosen = BTreeMap::<(String, String), (String, Vec<String>)>::new();
    for language in workspace.languages() {
        for krate in workspace
            .crates
            .iter()
            .filter(|krate| !krate.keys.is_empty())
        {
            let translations = match translations(krate, &language) {
                Ok(translations) => translations,
                Err(problem) => {
                    problems.push(problem);
                    continue;
                }
            };
            let Some(Translations { file, po }) = translations else {
                if complete.contains(&language) {
                    problems.push(format!("{}: no {language}.po", krate.locales().display()));
                }
                continue;
            };
            let place = file.display();
            let rule = match plural_rule(&po) {
                Ok(rule) => rule,
                Err(problem) => {
                    problems.push(format!("{place}: {problem}"));
                    continue;
                }
            };
            let mut seen = BTreeSet::new();
            for message in &po.messages {
                let id = key(message.context.as_deref(), &message.english);
                if !seen.insert(id.clone()) {
                    problems.push(format!("{place}: “{}” twice", message.english));
                }
                let Some(english) = krate.keys.iter().find(|candidate| candidate.id() == id) else {
                    problems.push(format!("{place}: “{}” is not in the code", message.english));
                    continue;
                };
                if english.plural != message.plural {
                    problems.push(format!(
                        "{place}: “{}” has another English plural in the code",
                        message.english
                    ));
                }
                if !message.translated() {
                    continue;
                }
                let wanted = if message.plural.is_some() {
                    rule.forms()
                } else {
                    1
                };
                if message.forms.len() != wanted {
                    problems.push(format!(
                        "{place}: “{}” needs {wanted} forms",
                        message.english
                    ));
                }
                let known = english.placeholders();
                for form in &message.forms {
                    match placeholders(form) {
                        Ok(used) => {
                            for name in used.into_iter().filter(|name| !known.contains(name)) {
                                problems.push(format!(
                                    "{place}: “{form}” uses {{{name}}}, which the English lacks"
                                ));
                            }
                        }
                        Err(problem) => problems.push(format!("{place}: {problem}")),
                    }
                }
            }
            if !regional(&language) {
                let (done, total) = counts.entry(language.clone()).or_default();
                *total += krate.keys.len();
                for english in &krate.keys {
                    let translated = po
                        .messages
                        .iter()
                        .find(|message| {
                            key(message.context.as_deref(), &message.english) == english.id()
                        })
                        .filter(|message| message.translated());
                    match translated {
                        Some(message) => {
                            *done += 1;
                            for program in workspace.programs_with(&krate.name) {
                                let slot =
                                    (program.clone(), format!("{language}\u{0}{}", english.id()));
                                match chosen.get(&slot) {
                                    Some((other, forms)) if *forms != message.forms => problems.push(format!(
                                        "{program}: {} and {other} translate “{}” differently in {language}; give one a context",
                                        krate.name, english.english
                                    )),
                                    Some(_) => {}
                                    None => {
                                        chosen.insert(slot, (krate.name.clone(), message.forms.clone()));
                                    }
                                }
                            }
                        }
                        None if complete.contains(&language) => {
                            problems
                                .push(format!("{place}: “{}” is not translated", english.english));
                        }
                        None => {}
                    }
                }
            }
        }
    }
    for (language, (done, total)) in &counts {
        println!("{language}: {done} of {total}");
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

fn update(workspace: &Workspace) -> Result<(), Vec<String>> {
    let mut problems = workspace.problems.clone();
    for language in workspace.languages() {
        let header = workspace
            .crates
            .iter()
            .find_map(|krate| translations(krate, &language).ok().flatten())
            .map(|translations| translations.po.header);
        for krate in workspace
            .crates
            .iter()
            .filter(|krate| !krate.keys.is_empty())
        {
            let existing = match translations(krate, &language) {
                Ok(existing) => existing,
                Err(problem) => {
                    problems.push(problem);
                    continue;
                }
            };
            if existing.is_none() && regional(&language) {
                continue;
            }
            let file = krate.locales().join(format!("{language}.po"));
            let (header, old) = match existing {
                Some(Translations { po, .. }) => (po.header, po.messages),
                None => (header.clone().unwrap_or_default(), Vec::new()),
            };
            let find = |english: &Key| {
                old.iter()
                    .find(|message| {
                        key(message.context.as_deref(), &message.english) == english.id()
                    })
                    .cloned()
            };
            let messages = krate
                .keys
                .iter()
                .filter_map(|english| match find(english) {
                    Some(message) => Some(Message {
                        plural: english.plural.clone(),
                        ..message
                    }),
                    None if regional(&language) => None,
                    None => Some(Message {
                        context: english.context.clone(),
                        english: english.english.clone(),
                        plural: english.plural.clone(),
                        ..Message::default()
                    }),
                })
                .collect();
            let written = std::fs::create_dir_all(krate.locales())
                .and_then(|()| std::fs::write(&file, po::write(&File { header, messages })));
            if let Err(error) = written {
                problems.push(format!("{}: {error}", file.display()));
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

fn pack(workspace: &Workspace, folder: &Path) -> Result<(), Vec<String>> {
    let mut problems = workspace.problems.clone();
    let development = &workspace.development;
    for program in &workspace.programs {
        let crates = workspace.crates_of(program);
        let keys = crates
            .iter()
            .flat_map(|krate| &krate.keys)
            .collect::<Vec<_>>();
        if keys.is_empty() {
            continue;
        }
        let entries = keys
            .iter()
            .map(|english| Entry {
                context: english.context.clone(),
                english: english.english.clone(),
                plural: english.plural.clone(),
                forms: [Some(english.english.clone()), english.plural.clone()]
                    .into_iter()
                    .flatten()
                    .collect(),
            })
            .collect::<Vec<_>>();
        let header = format!(
            "Language: {development}\nContent-Type: text/plain; charset=UTF-8\n{DEVELOPMENT_HEADER}: yes\n"
        );
        let target = folder
            .join("development")
            .join(&program.name)
            .join(format!("{development}.mo"));
        if let Err(problem) = write_catalog(&target, &header, &entries) {
            problems.push(problem);
        }
        for language in workspace.languages() {
            let mut header = None;
            let mut entries = Vec::<Entry>::new();
            for krate in &crates {
                let Some(Translations { po, .. }) = translations(krate, &language)
                    .map_err(|problem| problems.push(problem))
                    .ok()
                    .flatten()
                else {
                    continue;
                };
                header.get_or_insert_with(|| po.header.clone());
                entries.extend(po.messages.into_iter().filter(Message::translated).map(
                    |message| Entry {
                        context: message.context,
                        english: message.english,
                        plural: message.plural,
                        forms: message.forms,
                    },
                ));
            }
            let Some(header) = header.filter(|_| !entries.is_empty()) else {
                continue;
            };
            let target = folder
                .join("packs")
                .join(&language)
                .join(&program.name)
                .join(format!("{language}.mo"));
            if let Err(problem) = write_catalog(&target, &header, &entries) {
                problems.push(problem);
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

fn write_catalog(target: &Path, header: &str, entries: &[Entry]) -> Result<(), String> {
    target
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(target, catalog::write(header, entries)))
        .map_err(|error| format!("{}: {error}", target.display()))
}

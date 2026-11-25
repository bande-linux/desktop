mod arguments;
mod shown;

use bande_intents::{
    Client, Definition, Dict, Manifest, Manifests, Reason, Refusal, Returns, dict_to_json, domain,
};
use bande_l10n::tr;
use serde_json::{Value as Json, json};
use zbus::blocking::Connection;

use crate::arguments::{Given, given};

const JSON: &str = "--json";
const HELP: &str = "--help";
const UNDO: &str = "undo";
const PAGE: u32 = 100;
const PROPERTIES: &str = "properties";

/// Carries out the command line `words` with the programs of `manifests`, and gives what to
/// print: an answer, or an error.
pub fn run(
    manifests: &Manifests,
    connect: impl FnOnce() -> zbus::Result<Connection>,
    words: &[String],
) -> Result<String, String> {
    let json = words.iter().any(|word| word == JSON);
    let words: Vec<String> = words.iter().filter(|word| *word != JSON).cloned().collect();
    carry_out(manifests, connect, &words, json).map_err(|refusal| match json {
        true => json!({
            "error": refusal.reason.word(),
            "message": refusal.message,
            "parameter": refusal.parameter,
            "accepts": refusal.accepts,
            "candidates": refusal.candidates,
        })
        .to_string(),
        false => shown::error(&refusal),
    })
}

fn carry_out(
    manifests: &Manifests,
    connect: impl FnOnce() -> zbus::Result<Connection>,
    words: &[String],
    json: bool,
) -> Result<String, Refusal> {
    match words {
        [] => Ok(shown::everything(manifests)),
        [help] if help == HELP => Ok(shown::everything(manifests)),
        [undo, help] if undo == UNDO && help == HELP => Ok(shown::everything(manifests)),
        [of, help] if help == HELP => carry_out(manifests, connect, std::slice::from_ref(of), json),
        [undo, receipt @ ..] if undo == UNDO => {
            let connection = connect().map_err(Refusal::from)?;
            take_back(manifests, &Client::new(&connection), receipt, json)
        }
        [of, verb, rest @ ..] => {
            let id = format!("{of}.{}", verb.replace('-', "_"));
            let (manifest, definition) = manifests.intent(&id).ok_or_else(|| {
                let there = manifests.serving(of).is_some();
                unknown(&id, if there { of } else { "" })
            })?;
            if rest.iter().any(|word| word == HELP) {
                return Ok(shown::help(manifest, &id, definition));
            }
            let given = given(definition, rest)?;
            let connection = connect().map_err(Refusal::from)?;
            call(
                &Client::new(&connection),
                manifest,
                &id,
                definition,
                given,
                json,
            )
        }
        [of] => match manifests.serving(of) {
            Some(_) => Ok(shown::domain(manifests, of)),
            None => Err(unknown(of, "")),
        },
    }
}

fn unknown(id: &str, of: &str) -> Refusal {
    Refusal::new(
        Reason::UnknownIntent,
        match of {
            "" => tr!(
                "Nothing installed can do “{intent}”. See “bande”.",
                intent = id
            ),
            of => tr!(
                "Nothing installed can do “{intent}”. See “bande {domain}”.",
                intent = id,
                domain = of
            ),
        },
    )
}

fn take_back(
    manifests: &Manifests,
    client: &Client,
    receipt: &[String],
    json: bool,
) -> Result<String, Refusal> {
    let receipt = match receipt {
        [receipt] => receipt.clone(),
        [] => client
            .all_receipts(manifests)
            .into_iter()
            .next()
            .map(|newest| newest.receipt)
            .ok_or_else(|| Refusal::not_found(tr!("There is nothing to undo")))?,
        [_, more, ..] => return Err(Refusal::invalid(more, vec!["undo [receipt]".to_owned()])),
    };
    let summary = client.undo(manifests, &receipt)?;
    Ok(match json {
        true => json!({ "summary": summary, "receipt": receipt }).to_string(),
        false => summary,
    })
}

fn gives(definition: &Definition, id: &str, verb: &str) -> bool {
    let named = id
        .strip_prefix(domain(id))
        .and_then(|rest| rest.strip_prefix('.'))
        .and_then(|rest| rest.strip_prefix(verb))
        .is_some_and(|rest| rest.is_empty() || rest.starts_with('_'));
    named && matches!(definition.returns, Some(Returns::List(_)))
}

fn call(
    client: &Client,
    manifest: &Manifest,
    id: &str,
    definition: &Definition,
    given: Given,
    json: bool,
) -> Result<String, Refusal> {
    if definition.only_reads() && gives(definition, id, "find") {
        let things = find_all(client, &manifest.bus, id, &given.arguments)?;
        return Ok(things_text(&things, json, false));
    }
    if definition.only_reads() && gives(definition, id, "get") {
        let mut given = given;
        let references = given.take_references(definition);
        let properties = given.take(PROPERTIES);
        let things = client.get(
            &manifest.bus,
            id,
            &references,
            &properties,
            &given.arguments,
        )?;
        return Ok(things_text(&things, json, true));
    }
    let prepared = client.prepare(&manifest.bus, id, &given.arguments)?;
    let (summary, effects) = (prepared.summary.clone(), prepared.effects.clone());
    let performed = client.perform(prepared)?;
    if gives(definition, id, "find") {
        return Ok(things_text(&performed.things(), json, false));
    }
    Ok(match json {
        true => json!({
            "summary": summary,
            "effects": effects,
            "cancelled": performed.cancelled,
            "result": performed.json(),
        })
        .to_string(),
        false => shown::performed(&summary, &performed),
    })
}

fn find_all(client: &Client, bus: &str, id: &str, arguments: &Dict) -> Result<Vec<Dict>, Refusal> {
    let mut things = Vec::new();
    let mut cursor = String::new();
    loop {
        let page = client.find(bus, id, arguments, &cursor, PAGE)?;
        things.extend(page.things);
        cursor = page.cursor;
        if cursor.is_empty() {
            return Ok(things);
        }
    }
}

fn things_text(things: &[Dict], json: bool, whole: bool) -> String {
    match json {
        true => Json::Array(things.iter().map(dict_to_json).collect()).to_string(),
        false => shown::things(things, whole),
    }
}

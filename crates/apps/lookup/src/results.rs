use bande_dict::{Choices, Hit, Pack};
use bande_inbox::Outbox;
use bande_toolkit::Spacing;
use bande_toolkit::ui::{Axis, Label, Stack, Widget};

use crate::L10N;
use crate::online;

pub type Answer = Result<Vec<Hit>, String>;

pub fn enabled_sources() -> (Vec<Pack>, Vec<String>) {
    let languages = bande_dict::user_languages();
    let choices = Choices::load();
    let sources = choices
        .arrange(bande_dict::installed(), &languages)
        .into_iter()
        .filter(|source| choices.enabled(source, &languages))
        .collect();
    (sources, languages)
}

pub fn find(word: &str, hint: Option<String>, online: bool, answers: &Outbox<Answer>) {
    let (sources, languages) = enabled_sources();
    if sources.is_empty() {
        answers.send(Err(L10N.tr("no-sources")));
        return;
    }
    let word_languages: Vec<String> = hint.into_iter().chain(languages).collect();
    let local = bande_dict::look_up(&sources, word, &word_languages);
    let language = match &local {
        Ok(hits) if !hits.is_empty() => hits[0].language.clone(),
        _ => word_languages.first().cloned().unwrap_or_default(),
    };
    if !answers.send(local.map_err(|error| error.to_string())) || !online {
        return;
    }
    for (place, source) in sources.iter().enumerate() {
        if source.online.is_none() {
            continue;
        }
        if let Some(mut hit) = online::look_up(source, word, &language) {
            hit.place = place;
            if !answers.send(Ok(vec![hit])) {
                return;
            }
        }
    }
}

pub fn merge(answer: &mut Option<Answer>, arrived: Vec<Answer>) {
    for next in arrived {
        match (&mut *answer, next) {
            (Some(Ok(hits)), Ok(more)) => {
                hits.extend(more);
                hits.sort_by_key(|hit| hit.place);
            }
            (shown, next) => *shown = Some(next),
        }
    }
}

fn part_of_speech(pos: &str) -> String {
    match pos {
        "noun" | "verb" | "adj" | "adv" => L10N.tr(&format!("pos-{pos}")),
        other => other.to_owned(),
    }
}

pub fn compact(hits: &[Hit]) -> Vec<&Hit> {
    let mut kinds = Vec::new();
    hits.iter()
        .filter(|hit| {
            let first_of_kind = !kinds.contains(&hit.kind);
            kinds.push(hit.kind);
            first_of_kind
        })
        .collect()
}

fn hit_widgets(hit: &Hit) -> Vec<Box<dyn Widget>> {
    let mut widgets: Vec<Box<dyn Widget>> = vec![Box::new(Label::new(&hit.source).secondary())];
    if let Some((first, ending)) = &hit.derived {
        let note = L10N.tr_args(
            "derived",
            [
                ("first", first.as_str().into()),
                ("ending", ending.as_str().into()),
            ],
        );
        widgets.push(Box::new(Label::new(&note).secondary()));
    }
    for entry in &hit.entries {
        let title = match entry.pos.is_empty() {
            true => entry.word.clone(),
            false => format!("{} · {}", entry.word, part_of_speech(&entry.pos)),
        };
        widgets.push(Box::new(Label::new(&title).secondary()));
        for line in &entry.lines {
            let text = match line.kind {
                'O' => L10N.tr_args("form-of", [("word", line.text.as_str().into())]),
                _ => line.text.clone(),
            };
            widgets.push(Box::new(Label::new(&text)));
        }
    }
    widgets.push(Box::new(Label::new(&hit.attribution).secondary()));
    widgets
}

pub fn shown(word: &str, answer: Option<&Answer>, hits: Vec<&Hit>) -> Box<dyn Widget> {
    let mut parts: Vec<Box<dyn Widget>> = Vec::new();
    match answer {
        None => {}
        Some(Err(message)) => parts.push(Box::new(Label::new(message).secondary())),
        Some(Ok(all)) if all.is_empty() => {
            let message = L10N.tr_args("no-entries", [("word", word.into())]);
            parts.push(Box::new(Label::new(&message).secondary()));
        }
        Some(Ok(_)) => {
            for hit in hits {
                parts.push(Box::new(Stack::new(
                    Axis::Vertical,
                    Spacing::XS as f32,
                    0.0,
                    hit_widgets(hit),
                )));
            }
        }
    }
    Box::new(Stack::new(Axis::Vertical, Spacing::M as f32, 0.0, parts))
}

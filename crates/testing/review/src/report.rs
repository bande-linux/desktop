use std::error::Error;
use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use crate::recorder::VIDEO;
use crate::scenario::{Line, Step};

pub const SHOTS: &str = "shots";
const VIEWER: &str = include_str!("viewer.html");
const TIMELINE_SLOT: &str = "\"__TIMELINE__\"";
const TITLE_SLOT: &str = "__TITLE__";

pub enum Outcome {
    Done,
    TimedOut,
    Shot(String),
}

#[derive(Serialize)]
pub struct Entry {
    line: usize,
    source: String,
    kind: &'static str,
    text: Option<String>,
    start: f64,
    end: f64,
    status: String,
    shot: Option<String>,
}

impl Entry {
    pub fn new(
        line: &Line,
        start: Duration,
        end: Duration,
        outcome: Result<Outcome, Box<dyn Error>>,
    ) -> Self {
        let (kind, text) = match &line.step {
            Step::Key(_) => ("key", None),
            Step::Type(_) => ("type", None),
            Step::Move(_) => ("move", None),
            Step::Click { .. } => ("click", None),
            Step::Drag { .. } => ("drag", None),
            Step::Scroll { .. } => ("scroll", None),
            Step::Pinch { .. } => ("pinch", None),
            Step::Wait(_) => ("wait", None),
            Step::WaitForWindow { .. } => ("wait-for-window", None),
            Step::Shot(name) => ("shot", Some(name.clone())),
            Step::Note(note) => ("note", Some(note.clone())),
        };
        let (status, shot) = match outcome {
            Ok(Outcome::Done) => ("ok".to_owned(), None),
            Ok(Outcome::TimedOut) => ("timed out".to_owned(), None),
            Ok(Outcome::Shot(file)) => ("ok".to_owned(), Some(file)),
            Err(error) => (format!("failed: {error}"), None),
        };
        Self {
            line: line.number,
            source: line.source.clone(),
            kind,
            text,
            start: start.as_secs_f64(),
            end: end.as_secs_f64(),
            status,
            shot,
        }
    }
}

#[derive(Serialize)]
pub struct Timeline {
    title: String,
    scenario: String,
    video: &'static str,
    width: u32,
    height: u32,
    duration: f64,
    completed: bool,
    steps: Vec<Entry>,
}

impl Timeline {
    pub fn new(
        path: &Path,
        source: &str,
        (width, height): (u32, u32),
        duration: Duration,
        steps: Vec<Entry>,
        completed: bool,
    ) -> Self {
        Self {
            title: path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_default(),
            scenario: source.to_owned(),
            video: VIDEO,
            width,
            height,
            duration: duration.as_secs_f64(),
            completed,
            steps,
        }
    }
}

pub fn shot_file(number: usize, name: &str) -> String {
    let mut slug = String::new();
    for character in name.chars().flat_map(char::to_lowercase) {
        if character.is_ascii_alphanumeric() {
            slug.push(character);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_end_matches('-');
    format!("{SHOTS}/{number:02}-{slug}.png")
}

fn viewer(timeline: &Timeline) -> Result<String, serde_json::Error> {
    let data = serde_json::to_string(timeline)?.replace("</", "<\\/");
    let title = timeline
        .title
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    Ok(VIEWER
        .replace(TITLE_SLOT, &title)
        .replace(TIMELINE_SLOT, &data))
}

pub fn write(out: &Path, timeline: &Timeline) -> Result<(), Box<dyn Error>> {
    std::fs::write(
        out.join("timeline.json"),
        serde_json::to_string_pretty(timeline)?,
    )?;
    std::fs::write(out.join("index.html"), viewer(timeline)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shot_files_are_numbered_and_readable() {
        assert_eq!(
            shot_file(3, "Launcher open: Terminal!"),
            "shots/03-launcher-open-terminal.png"
        );
        assert_eq!(shot_file(12, "  dark mode  "), "shots/12-dark-mode.png");
    }

    #[test]
    fn the_viewer_embeds_the_timeline_safely() {
        let timeline = Timeline::new(
            Path::new("ci/review/scenarios/terminal.review"),
            "note \"</script><b>\"\n",
            (100, 50),
            Duration::from_secs(2),
            Vec::new(),
            true,
        );

        let page = viewer(&timeline).unwrap();

        assert!(page.contains("<title>terminal"));
        assert!(!page.contains(TIMELINE_SLOT));
        assert!(!page.contains("</script><b>"));
        assert!(page.contains(r#""video":"review.mp4""#));
    }
}

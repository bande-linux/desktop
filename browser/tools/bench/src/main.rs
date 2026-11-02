use std::borrow::Cow;
use std::fmt::Write;
use std::path::Path;
use std::process::ExitCode;
use std::time::{Duration, Instant};

use web_content::{Fonts, Page};
use web_css::{ParseResult, Parser, RuleHandler, RuleListParser, Tokenizer};

const USAGE: &str = "usage: bench [FILE.html | FILE.css ...] (default: a generated article)";
const VIEWPORT_WIDTH: f32 = 1280.0;

fn main() -> ExitCode {
    let paths: Vec<String> = std::env::args().skip(1).collect();
    if paths.iter().any(|arg| arg.starts_with('-')) {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    }
    let mut fonts = Fonts::new();
    if paths.is_empty() {
        html("generated article", &article(2_000), &mut fonts);
        css("generated stylesheet", &stylesheet(2_000));
        return ExitCode::SUCCESS;
    }
    for path in &paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            eprintln!("bench: cannot read {path}");
            return ExitCode::FAILURE;
        };
        if Path::new(path).extension().is_some_and(|ext| ext == "css") {
            css(path, &text);
        } else {
            html(path, &text, &mut fonts);
        }
    }
    ExitCode::SUCCESS
}

fn html(name: &str, source: &str, fonts: &mut Fonts) {
    let before = anonymous_bytes();
    let page = Page::new(web_content::parse(source.as_bytes()), VIEWPORT_WIDTH, fonts);
    let process = anonymous_bytes().saturating_sub(before);
    let parse = throughput(source.len(), || drop(web_content::parse(source.as_bytes())));
    let pipeline = throughput(source.len(), || {
        drop(Page::new(
            web_content::parse(source.as_bytes()),
            VIEWPORT_WIDTH,
            fonts,
        ))
    });
    let memory = page.memory();
    let elements = memory.elements.max(1);
    let total = memory.dom + memory.styles + memory.boxes + memory.fragments + memory.display_list;
    println!(
        "{name}: {} bytes, {} elements",
        source.len(),
        memory.elements
    );
    println!("  HTML parse            {parse:8.1} MB/s");
    println!("  parse, style, layout  {pipeline:8.1} MB/s");
    println!(
        "  bytes per element: DOM {}, styles {}, boxes {}, fragments {}, display list {}, total {}, process {}",
        memory.dom / elements,
        memory.styles / elements,
        memory.boxes / elements,
        memory.fragments / elements,
        memory.display_list / elements,
        total / elements,
        process / elements,
    );
}

fn css(name: &str, source: &str) {
    let parse = throughput(source.len(), || {
        let mut tokenizer = Tokenizer::new(source);
        let mut input = Parser::new(&mut tokenizer);
        RuleListParser::new_for_stylesheet(&mut input, &mut Consume).for_each(drop);
    });
    println!("{name}: {} bytes", source.len());
    println!("  CSS parse             {parse:8.1} MB/s");
}

struct Consume;

fn consume(input: &mut Parser<'_, '_>) -> ParseResult<()> {
    while input.next_including_whitespace().is_ok() {}
    Ok(())
}

impl<'i> RuleHandler<'i> for Consume {
    type AtPrelude = ();
    type QualifiedPrelude = ();
    type Item = ();

    fn at_rule_prelude(
        &mut self,
        _name: Cow<'i, str>,
        input: &mut Parser<'i, '_>,
    ) -> ParseResult<()> {
        consume(input)
    }

    fn at_rule_block(&mut self, _prelude: (), input: &mut Parser<'i, '_>) -> ParseResult<()> {
        consume(input)
    }

    fn at_rule_without_block(&mut self, _prelude: ()) -> ParseResult<()> {
        Ok(())
    }

    fn qualified_rule_prelude(&mut self, input: &mut Parser<'i, '_>) -> ParseResult<()> {
        consume(input)
    }

    fn qualified_rule_block(
        &mut self,
        _prelude: (),
        input: &mut Parser<'i, '_>,
    ) -> ParseResult<()> {
        consume(input)
    }

    fn declaration(&mut self, _name: Cow<'i, str>, input: &mut Parser<'i, '_>) -> ParseResult<()> {
        consume(input)
    }
}

fn throughput(bytes: usize, mut run: impl FnMut()) -> f64 {
    let start = Instant::now();
    let mut best = Duration::MAX;
    let mut rounds = 0;
    while rounds < 5 || start.elapsed() < Duration::from_secs(2) {
        let round = Instant::now();
        run();
        best = best.min(round.elapsed());
        rounds += 1;
    }
    bytes as f64 / best.as_secs_f64().max(f64::EPSILON) / 1e6
}

fn anonymous_bytes() -> usize {
    std::fs::read_to_string("/proc/self/smaps_rollup")
        .ok()
        .and_then(|text| {
            text.lines()
                .find_map(|line| line.strip_prefix("Anonymous:"))
                .and_then(|value| value.trim().trim_end_matches("kB").trim().parse().ok())
        })
        .map_or(0, |kilobytes: usize| kilobytes * 1024)
}

fn article(sections: usize) -> String {
    let mut html = String::from(
        "<!DOCTYPE html><html lang=en><head><meta charset=utf-8><title>Article</title></head><body><main>",
    );
    for section in 0..sections {
        let _ = write!(
            html,
            "<h2 id=s{section}>Section {section}</h2><p class=lead>Some <a href=\"#s{section}\">linked</a> text with <em>emphasis</em>, <code>code</code> and &amp; entities.</p><ul><li>One</li><li>Two</li></ul>"
        );
    }
    html.push_str("</main></body></html>");
    html
}

fn stylesheet(rules: usize) -> String {
    let mut css = String::new();
    for rule in 0..rules {
        let _ = write!(
            css,
            ".item-{rule} > a:hover, #main ul li:nth-child(2n+1) {{ color: rgb(10 20 30 / 50%); margin: 0 auto {rule}px; font: 14px/1.4 \"Inter\", sans-serif }}\n@media (min-width: {rule}px) {{ .col-{rule} {{ width: calc(100% - 2em) }} }}\n"
        );
    }
    css
}

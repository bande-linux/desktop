use std::fmt::Write;
use std::path::Path;

use crate::engine;
use crate::report::Tally;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Namespace {
    Html,
    Svg,
    MathMl,
}

pub enum Node {
    Doctype {
        name: String,
        public_id: Option<String>,
        system_id: Option<String>,
    },
    Element {
        namespace: Namespace,
        name: String,
        attributes: Vec<(String, String)>,
        children: Vec<Node>,
        template_contents: Option<Vec<Node>>,
    },
    Text(String),
    Comment(String),
    ProcessingInstruction {
        target: String,
        data: String,
    },
}

pub struct Context<'a> {
    pub namespace: Namespace,
    pub name: &'a str,
}

pub fn run(wpt: &Path) -> Result<Tally, String> {
    let mut tally = Tally::new("html5lib tree-construction");
    for path in crate::files(&wpt.join("html/syntax/parsing/resources"), "dat")? {
        let text = std::fs::read_to_string(&path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let name = crate::file_name(&path);
        for (index, case) in cases(&text).into_iter().enumerate() {
            let context = case
                .fragment
                .map(|fragment| match fragment.split_once(' ') {
                    Some(("svg", name)) => Context {
                        namespace: Namespace::Svg,
                        name,
                    },
                    Some(("math", name)) => Context {
                        namespace: Namespace::MathMl,
                        name,
                    },
                    _ => Context {
                        namespace: Namespace::Html,
                        name: fragment,
                    },
                });
            let mut dump = String::new();
            let passed = crate::guarded(|| {
                let nodes = engine::parse(case.data, context.as_ref(), case.scripting);
                serialize(&nodes, 0, &mut dump);
                dump.trim_end_matches('\n') == case.document
            });
            tally.record(passed, || {
                format!(
                    "{name} #{}: {:?}\nexpected:\n{}\nactual:\n{}",
                    index + 1,
                    case.data,
                    case.document,
                    dump.trim_end_matches('\n')
                )
            });
        }
    }
    Ok(tally)
}

struct Case<'a> {
    data: &'a str,
    fragment: Option<&'a str>,
    scripting: bool,
    document: &'a str,
}

fn cases(text: &str) -> Vec<Case<'_>> {
    let mut cases = Vec::new();
    let mut case: Option<Case> = None;
    let mut section = "";
    let mut start = 0;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let header = line
            .trim_end_matches(['\n', '\r'])
            .strip_prefix('#')
            .filter(|name| *name == "data" || SECTIONS.contains(name));
        if let Some(name) = header {
            if let Some(case) = case.as_mut() {
                assign(case, section, &text[start..offset]);
            }
            if name == "data" {
                cases.extend(case.take());
                case = Some(Case {
                    data: "",
                    fragment: None,
                    scripting: true,
                    document: "",
                });
            }
            section = name;
            start = offset + line.len();
        }
        offset += line.len();
    }
    if let Some(mut case) = case {
        assign(&mut case, section, &text[start..]);
        cases.push(case);
    }
    cases
}

const SECTIONS: [&str; 7] = [
    "errors",
    "new-errors",
    "document-fragment",
    "script-off",
    "script-on",
    "document",
    "description",
];

fn assign<'a>(case: &mut Case<'a>, section: &str, content: &'a str) {
    let content = content.strip_suffix('\n').unwrap_or(content);
    match section {
        "data" => case.data = content,
        "document-fragment" => case.fragment = Some(content),
        "script-off" => case.scripting = false,
        "script-on" => case.scripting = true,
        "document" => case.document = content.trim_end_matches(['\n', '\r']),
        _ => {}
    }
}

fn serialize(nodes: &[Node], depth: usize, out: &mut String) {
    for node in nodes {
        let indent = "  ".repeat(depth);
        match node {
            Node::Doctype {
                name,
                public_id,
                system_id,
            } => {
                if public_id.is_none() && system_id.is_none() {
                    let _ = writeln!(out, "| {indent}<!DOCTYPE {name}>");
                } else {
                    let _ = writeln!(
                        out,
                        "| {indent}<!DOCTYPE {name} \"{}\" \"{}\">",
                        public_id.as_deref().unwrap_or_default(),
                        system_id.as_deref().unwrap_or_default()
                    );
                }
            }
            Node::Element {
                namespace,
                name,
                attributes,
                children,
                template_contents,
            } => {
                let prefix = match namespace {
                    Namespace::Html => "",
                    Namespace::Svg => "svg ",
                    Namespace::MathMl => "math ",
                };
                let _ = writeln!(out, "| {indent}<{prefix}{name}>");
                let mut sorted: Vec<_> = attributes.iter().collect();
                sorted.sort();
                for (key, value) in sorted {
                    let _ = writeln!(out, "| {indent}  {key}=\"{value}\"");
                }
                if let Some(contents) = template_contents {
                    let _ = writeln!(out, "| {indent}  content");
                    serialize(contents, depth + 2, out);
                }
                serialize(children, depth + 1, out);
            }
            Node::Text(text) => {
                let _ = writeln!(out, "| {indent}\"{text}\"");
            }
            Node::Comment(text) => {
                let _ = writeln!(out, "| {indent}<!-- {text} -->");
            }
            Node::ProcessingInstruction { target, data } => {
                let _ = writeln!(out, "| {indent}<?{target} {data}?>");
            }
        }
    }
}

use serde_json::{Value, json};

use crate::signature::Frame;

#[derive(Default)]
pub struct Image {
    pub name: String,
    pub build_id: Option<String>,
    pub package: Option<String>,
}

fn readable(symbol: &str) -> String {
    format!("{:#}", rustc_demangle::demangle(symbol))
}

pub struct ParsedFrame {
    pub symbol: Option<String>,
    pub module: String,
    pub offset: u64,
}

impl ParsedFrame {
    pub fn as_frame(&self) -> Frame<'_> {
        Frame {
            symbol: self.symbol.as_deref(),
            module: &self.module,
            offset: self.offset,
        }
    }
}

pub struct Thread {
    pub id: u64,
    pub crashed: bool,
    pub frames: Vec<ParsedFrame>,
}

#[derive(Default)]
pub struct Stacks {
    pub threads: Vec<Thread>,
    pub images: Vec<Image>,
}

fn base_name(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn parse_module(line: &str) -> Option<Image> {
    let rest = line.strip_prefix("Module ")?;
    let (name, rest) = rest
        .split_once(' ')
        .unwrap_or((rest.trim_end_matches('.'), ""));
    let mut image = Image {
        name: base_name(name).to_owned(),
        ..Image::default()
    };
    if let Some(id) = rest.split("with build-id ").nth(1) {
        image.build_id = id.split_whitespace().next().map(str::to_owned);
    }
    if let Some(package) = rest.split("from ").nth(1) {
        image.package = Some(package.trim().to_owned());
    }
    Some(image)
}

fn parse_coredump_frame(line: &str) -> Option<ParsedFrame> {
    let rest = line.trim_start().strip_prefix('#')?;
    let (_, rest) = rest.split_once(' ')?;
    let rest = rest.trim_start();
    let (_, rest) = rest.split_once(' ')?;
    let (symbol, place) = rest.rsplit_once(" (")?;
    let (module, offset) = place.trim_end_matches(')').split_once(" + ")?;
    let symbol = symbol.trim();
    Some(ParsedFrame {
        symbol: (symbol != "n/a").then(|| readable(symbol)),
        module: base_name(module).to_owned(),
        offset: u64::from_str_radix(offset.trim_start_matches("0x"), 16).ok()?,
    })
}

pub fn parse_coredump(message: &str, crashed_tid: Option<u64>) -> Stacks {
    let mut stacks = Stacks::default();
    for line in message.lines() {
        let trimmed = line.trim();
        if let Some(image) = parse_module(trimmed) {
            stacks.images.push(image);
        } else if let Some(id) = trimmed
            .strip_prefix("Stack trace of thread ")
            .and_then(|id| id.trim_end_matches(':').parse().ok())
        {
            let crashed = crashed_tid.map_or(stacks.threads.is_empty(), |tid| tid == id);
            stacks.threads.push(Thread {
                id,
                crashed,
                frames: Vec::new(),
            });
        } else if let Some(frame) = parse_coredump_frame(trimmed)
            && let Some(thread) = stacks.threads.last_mut()
        {
            thread.frames.push(frame);
        }
    }
    stacks
}

fn parse_eu_frame(line: &str, images: &mut Vec<Image>) -> Option<ParsedFrame> {
    let rest = line.trim_start().strip_prefix('#')?;
    let mut parts = rest.split_whitespace();
    let _number = parts.next()?;
    let address = parts.next()?;
    let address = u64::from_str_radix(address.trim_start_matches("0x"), 16).ok()?;
    let mut symbol = None;
    let mut module = String::new();
    let mut build_id = None;
    let mut words = parts.peekable();
    if let Some(word) = words.peek()
        && *word != "-"
    {
        symbol = Some(readable((*word).split('+').next().unwrap_or(word)));
        words.next();
    }
    while let Some(word) = words.next() {
        match word {
            "-" => module = base_name(words.next().unwrap_or_default()).to_owned(),
            _ if word.len() >= 16 && word.bytes().all(|b| b.is_ascii_hexdigit()) => {
                build_id = Some(word.to_owned());
            }
            _ => {}
        }
    }
    if !module.is_empty() && !images.iter().any(|image| image.name == module) {
        images.push(Image {
            name: module.clone(),
            build_id,
            package: None,
        });
    }
    Some(ParsedFrame {
        symbol,
        module,
        offset: address,
    })
}

pub fn parse_eu_stack(text: &str, main_tid: u64) -> Stacks {
    let mut stacks = Stacks::default();
    for line in text.lines() {
        if let Some(id) = line
            .strip_prefix("TID ")
            .and_then(|id| id.trim_end_matches(':').parse().ok())
        {
            stacks.threads.push(Thread {
                id,
                crashed: id == main_tid,
                frames: Vec::new(),
            });
        } else if let Some(frame) = parse_eu_frame(line, &mut stacks.images)
            && let Some(thread) = stacks.threads.last_mut()
        {
            thread.frames.push(frame);
        }
    }
    stacks
}

impl Stacks {
    pub fn faulting(&self) -> Vec<Frame<'_>> {
        self.threads
            .iter()
            .find(|thread| thread.crashed)
            .map(|thread| thread.frames.iter().map(ParsedFrame::as_frame).collect())
            .unwrap_or_default()
    }

    pub fn threads_json(&self) -> Value {
        Value::Array(
            self.threads
                .iter()
                .map(|thread| {
                    let frames: Vec<Value> = thread
                        .frames
                        .iter()
                        .map(|frame| {
                            json!({
                                "symbol": frame.symbol,
                                "module": frame.module,
                                "offset": frame.offset,
                            })
                        })
                        .collect();
                    json!({"id": thread.id, "faulting": thread.crashed, "frames": frames})
                })
                .collect(),
        )
    }

    pub fn images_json(&self) -> Value {
        Value::Array(
            self.images
                .iter()
                .map(|image| {
                    json!({
                        "name": image.name,
                        "build_id": image.build_id,
                        "package": image.package,
                    })
                })
                .collect(),
        )
    }

    pub fn build_ids(&self) -> Value {
        Value::Array(
            self.images
                .iter()
                .filter_map(|image| image.build_id.as_deref())
                .map(|id| json!(id))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::parse_coredump;

    #[test]
    fn a_systemd_coredump_message_becomes_threads_and_images() {
        let message = "Process 42 (bande-shell) of user 1000 dumped core.\n\nModule /usr/bin/bande-shell with build-id abc123\nModule libc.so.6 from deb glibc-2.41-12\nStack trace of thread 42:\n#0  0x0000ffff8a7b1234 __pthread_kill_implementation (libc.so.6 + 0x81234)\n#1  0x0000aaaa00001000 n/a (bande-shell + 0x1000)\n\nStack trace of thread 43:\n#0  0x0000ffff8a7b9999 n/a (libc.so.6 + 0x99)\nELF object binary architecture: ARM AArch64\n";
        let stacks = parse_coredump(message, None);
        assert_eq!(stacks.threads.len(), 2);
        assert!(stacks.threads[0].crashed && !stacks.threads[1].crashed);
        assert_eq!(stacks.threads[0].frames.len(), 2);
        assert_eq!(stacks.threads[0].frames[1].offset, 0x1000);
        assert_eq!(stacks.threads[0].frames[1].symbol, None);
        assert_eq!(stacks.images[0].name, "bande-shell");
        assert_eq!(stacks.images[0].build_id.as_deref(), Some("abc123"));
    }
}

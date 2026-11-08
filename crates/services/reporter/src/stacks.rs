use serde_json::{Value, json};

use crate::elfcore::build_id_of_file;
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
    let (number, rest) = rest.split_once(' ')?;
    let caller = number.parse::<u32>().ok()? > 0;
    let rest = rest.trim_start();
    let (_, rest) = rest.split_once(' ')?;
    let (symbol, place) = rest.rsplit_once(" (")?;
    let (module, offset) = place.trim_end_matches(')').split_once(" + ")?;
    let symbol = symbol.trim();
    let offset = u64::from_str_radix(offset.trim_start_matches("0x"), 16).ok()?;
    Some(ParsedFrame {
        symbol: (symbol != "n/a").then(|| readable(symbol)),
        module: base_name(module).to_owned(),
        offset: if caller {
            offset.saturating_sub(1)
        } else {
            offset
        },
    })
}

const SYSTEM_FOLDERS: [&str; 3] = ["/usr/", "/lib/", "/lib64/"];

impl Stacks {
    pub fn add_mapped_files(&mut self, files: &[String]) {
        for file in files {
            if !SYSTEM_FOLDERS.iter().any(|folder| file.starts_with(folder)) {
                continue;
            }
            let Some(build_id) = build_id_of_file(file) else {
                continue;
            };
            let name = base_name(file).to_owned();
            match self.images.iter_mut().find(|image| image.name == name) {
                Some(image) => image.build_id = Some(build_id),
                None => self.images.push(Image {
                    name,
                    build_id: Some(build_id),
                    package: None,
                }),
            }
        }
    }
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

fn parse_eu_frame(line: &str) -> Option<ParsedFrame> {
    let rest = line.trim_start().strip_prefix('#')?;
    let mut parts = rest.split_whitespace();
    let _number = parts.next()?;
    let address = parts.next()?;
    let address = u64::from_str_radix(address.trim_start_matches("0x"), 16).ok()?;
    let mut words = parts.peekable();
    let symbol = words
        .next_if(|word| *word != "-")
        .map(|word| readable(word.split('+').next().unwrap_or(word)));
    let module = match (words.next(), words.next()) {
        (Some("-"), Some(path)) => base_name(path).to_owned(),
        _ => String::new(),
    };
    Some(ParsedFrame {
        symbol,
        module,
        offset: address,
    })
}

fn parse_eu_place(line: &str) -> Option<(String, u64)> {
    let (build_id, place) = line.trim().strip_prefix('[')?.split_once("]@")?;
    let (_, offset) = place.split_once('+')?;
    let offset = u64::from_str_radix(offset.trim().trim_start_matches("0x"), 16).ok()?;
    Some((build_id.to_owned(), offset))
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
        } else if let Some(frame) = parse_eu_frame(line)
            && let Some(thread) = stacks.threads.last_mut()
        {
            thread.frames.push(frame);
        } else if let Some((build_id, offset)) = parse_eu_place(line)
            && let Some(frame) = stacks
                .threads
                .last_mut()
                .and_then(|thread| thread.frames.last_mut())
            && !frame.module.is_empty()
        {
            frame.offset = offset;
            if !stacks.images.iter().any(|image| image.name == frame.module) {
                stacks.images.push(Image {
                    name: frame.module.clone(),
                    build_id: (!build_id.is_empty()).then_some(build_id),
                    package: None,
                });
            }
        }
    }
    for thread in &mut stacks.threads {
        thread.frames.retain(|frame| {
            !frame.module.is_empty() && stacks.images.iter().any(|image| image.name == frame.module)
        });
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

    fn frames_json(&self, frames: &[ParsedFrame]) -> Vec<Value> {
        frames
            .iter()
            .map(|frame| {
                json!({
                    "symbol": frame.symbol,
                    "module": frame.module,
                    "build_id": self.build_id_of(&frame.module),
                    "offset": frame.offset,
                })
            })
            .collect()
    }

    pub fn faulting_frames_json(&self) -> Vec<Value> {
        self.threads
            .iter()
            .find(|thread| thread.crashed)
            .map(|thread| self.frames_json(&thread.frames))
            .unwrap_or_default()
    }

    pub fn threads_json(&self) -> Value {
        Value::Array(
            self.threads
                .iter()
                .map(|thread| {
                    json!({
                        "id": thread.id,
                        "faulting": thread.crashed,
                        "frames": self.frames_json(&thread.frames),
                    })
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

    fn build_id_of(&self, module: &str) -> Option<&str> {
        self.images
            .iter()
            .find(|image| image.name == module)
            .and_then(|image| image.build_id.as_deref())
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
    use super::{parse_coredump, parse_eu_stack};

    #[test]
    fn eu_stack_frames_keep_the_offset_in_their_module() {
        let text = "PID 7 - process\nTID 7:\n#0  0x00007fffbe3a1b5c clock_nanosleep - /usr/lib/aarch64-linux-gnu/libc.so.6\n    [405d95c1]@0x7fffbe2e0000+0xc1b5b\n#1  0x000055555cd26f88 - /usr/bin/sleep\n    [53c7c062]@0x55555cd20000+0x6f87\n    src/sleep.c:42\n#2  0x0000000000001234\n";
        let stacks = parse_eu_stack(text, 7);
        let frames = &stacks.threads[0].frames;
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].offset, 0xc1b5b);
        assert_eq!(frames[1].module, "sleep");
        assert_eq!(frames[1].offset, 0x6f87);
        assert_eq!(stacks.build_id_of("libc.so.6"), Some("405d95c1"));
    }

    #[test]
    fn a_systemd_coredump_message_becomes_threads_and_images() {
        let message = "Process 42 (bande-shell) of user 1000 dumped core.\n\nModule /usr/bin/bande-shell with build-id abc123\nModule libc.so.6 from deb glibc-2.41-12\nStack trace of thread 42:\n#0  0x0000ffff8a7b1234 __pthread_kill_implementation (libc.so.6 + 0x81234)\n#1  0x0000aaaa00001000 n/a (bande-shell + 0x1000)\n\nStack trace of thread 43:\n#0  0x0000ffff8a7b9999 n/a (libc.so.6 + 0x99)\nELF object binary architecture: ARM AArch64\n";
        let stacks = parse_coredump(message, None);
        assert_eq!(stacks.threads.len(), 2);
        assert!(stacks.threads[0].crashed && !stacks.threads[1].crashed);
        assert_eq!(stacks.threads[0].frames.len(), 2);
        assert_eq!(stacks.threads[0].frames[1].offset, 0xfff);
        assert_eq!(stacks.threads[0].frames[1].symbol, None);
        assert_eq!(stacks.images[0].name, "bande-shell");
        assert_eq!(stacks.images[0].build_id.as_deref(), Some("abc123"));
    }
}

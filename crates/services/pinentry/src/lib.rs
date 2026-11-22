use std::io::{self, BufRead, Write};

use zeroize::Zeroizing;

const LONGEST_LINE: usize = 1000;
const LONGEST_DATA: usize = 900;
const CANCELED: &str = "ERR 83886179 Operation cancelled <Pinentry>";
const NOT_CONFIRMED: &str = "ERR 83886194 Not confirmed <Pinentry>";
const UNKNOWN_COMMAND: &str = "ERR 536871187 Unknown IPC command <User defined source 1>";
const LINE_TOO_LONG: &str = "ERR 536871107 Line too long <User defined source 1>";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Texts {
    pub title: Option<String>,
    pub description: Option<String>,
    pub label: Option<String>,
    pub error: Option<String>,
    pub ok: Option<String>,
    pub cancel: Option<String>,
    pub not_ok: Option<String>,
    pub repeat: Option<String>,
    pub mismatch: Option<String>,
}

impl Texts {
    pub fn lines(&self) -> Vec<(&'static str, &str)> {
        [
            ("title", &self.title),
            ("description", &self.description),
            ("label", &self.label),
            ("error", &self.error),
            ("ok", &self.ok),
            ("cancel", &self.cancel),
            ("not-ok", &self.not_ok),
            ("repeat", &self.repeat),
            ("mismatch", &self.mismatch),
        ]
        .into_iter()
        .filter_map(|(key, value)| Some((key, value.as_deref()?)))
        .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Passphrase,
    Confirm,
    Message,
}

impl Kind {
    pub fn flag(self) -> &'static str {
        match self {
            Self::Passphrase => "--passphrase",
            Self::Confirm => "--confirm",
            Self::Message => "--message",
        }
    }
}

pub enum Answer {
    Passphrase(Zeroizing<String>),
    Confirmed,
    NotConfirmed,
    Cancelled,
}

pub fn decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escaped = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match escaped {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

fn data_lines(data: &str) -> Zeroizing<String> {
    let mut lines = Zeroizing::new(String::new());
    let mut length = 0;
    for character in data.chars() {
        if length == 0 {
            lines.push_str("D ");
        }
        match character {
            '%' => lines.push_str("%25"),
            '\r' => lines.push_str("%0D"),
            '\n' => lines.push_str("%0A"),
            character => lines.push(character),
        }
        length += character.len_utf8();
        if length >= LONGEST_DATA {
            lines.push('\n');
            length = 0;
        }
    }
    if length > 0 {
        lines.push('\n');
    }
    lines
}

struct Session<W: Write> {
    output: W,
    texts: Texts,
}

impl<W: Write> Session<W> {
    fn send(&mut self, line: &str) -> io::Result<()> {
        self.output.write_all(line.as_bytes())?;
        if !line.ends_with('\n') {
            self.output.write_all(b"\n")?;
        }
        self.output.flush()
    }

    fn command(
        &mut self,
        line: &str,
        ask: &mut impl FnMut(Kind, &Texts) -> Answer,
    ) -> io::Result<bool> {
        let (command, argument) = line.split_once(' ').unwrap_or((line, ""));
        let argument = decoded(argument);
        let text = (!argument.is_empty()).then(|| argument.clone());
        let texts = &mut self.texts;
        let field = match command.to_ascii_uppercase().as_str() {
            "SETTITLE" => Some(&mut texts.title),
            "SETDESC" => Some(&mut texts.description),
            "SETPROMPT" => Some(&mut texts.label),
            "SETERROR" => Some(&mut texts.error),
            "SETOK" => Some(&mut texts.ok),
            "SETCANCEL" => Some(&mut texts.cancel),
            "SETNOTOK" => Some(&mut texts.not_ok),
            "SETREPEAT" => {
                texts.repeat = Some(text.clone().unwrap_or_default());
                None
            }
            "SETREPEATERROR" => Some(&mut texts.mismatch),
            "GETPIN" => return self.ask(Kind::Passphrase, ask).map(|()| true),
            "CONFIRM" => {
                let kind = match argument.trim() {
                    "--one-button" => Kind::Message,
                    _ => Kind::Confirm,
                };
                return self.ask(kind, ask).map(|()| true);
            }
            "MESSAGE" => return self.ask(Kind::Message, ask).map(|()| true),
            "GETINFO" => {
                let info = match argument.trim() {
                    "flavor" => Some("bande".to_owned()),
                    "version" => Some(env!("CARGO_PKG_VERSION").to_owned()),
                    "pid" => Some(std::process::id().to_string()),
                    "ttyinfo" => Some("- - -".to_owned()),
                    _ => None,
                };
                match info {
                    Some(info) => self.send(&data_lines(&info))?,
                    None => return self.send(UNKNOWN_COMMAND).map(|()| true),
                }
                None
            }
            "RESET" => {
                self.texts = Texts::default();
                None
            }
            "BYE" => {
                self.send("OK closing connection")?;
                return Ok(false);
            }
            "OPTION" | "NOP" | "SETKEYINFO" | "SETTIMEOUT" | "SETQUALITYBAR"
            | "SETQUALITYBAR_TT" | "SETGENPIN" | "SETGENPIN_TT" | "SETREPEATOK"
            | "CLEARPASSPHRASE" => None,
            _ => return self.send(UNKNOWN_COMMAND).map(|()| true),
        };
        if let Some(field) = field {
            *field = text;
        }
        self.send("OK").map(|()| true)
    }

    fn ask(&mut self, kind: Kind, ask: &mut impl FnMut(Kind, &Texts) -> Answer) -> io::Result<()> {
        let answer = ask(kind, &self.texts);
        self.texts.error = None;
        match (kind, answer) {
            (Kind::Passphrase, Answer::Passphrase(passphrase)) => {
                if self.texts.repeat.is_some() {
                    self.send("S PIN_REPEATED")?;
                }
                if !passphrase.is_empty() {
                    let lines = data_lines(&passphrase);
                    self.output.write_all(lines.as_bytes())?;
                }
                self.send("OK")
            }
            (Kind::Confirm | Kind::Message, Answer::Confirmed) => self.send("OK"),
            (Kind::Confirm, Answer::NotConfirmed) => self.send(NOT_CONFIRMED),
            _ => self.send(CANCELED),
        }
    }
}

pub fn serve(
    mut input: impl BufRead,
    output: impl Write,
    mut ask: impl FnMut(Kind, &Texts) -> Answer,
) -> io::Result<()> {
    let mut session = Session {
        output,
        texts: Texts::default(),
    };
    session.send("OK Pleased to meet you")?;
    loop {
        let mut line = Zeroizing::new(Vec::new());
        let read =
            io::Read::take(&mut input, LONGEST_LINE as u64 + 1).read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(());
        }
        if line.len() > LONGEST_LINE {
            session.send(LINE_TOO_LONG)?;
            while !line.ends_with(b"\n") && !line.is_empty() {
                line.clear();
                io::Read::take(&mut input, LONGEST_LINE as u64).read_until(b'\n', &mut line)?;
            }
            continue;
        }
        let text = String::from_utf8_lossy(&line);
        let text = text.trim_end_matches(['\r', '\n']);
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        if !session.command(text, &mut ask)? {
            return Ok(());
        }
    }
}

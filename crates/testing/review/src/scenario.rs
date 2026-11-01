use std::fmt;
use std::time::Duration;

use crate::keys::{Chord, character_key, chord};

pub const MAX_FINGERS: u32 = 5;
const DEFAULT_WINDOW_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    Left,
    Right,
    Middle,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pinch {
    In,
    Out,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub x: u32,
    pub y: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Key(Vec<Chord>),
    Type(String),
    Move(Point),
    Click {
        button: Button,
        count: u8,
        at: Option<Point>,
    },
    Drag {
        from: Point,
        to: Point,
    },
    Scroll {
        direction: Direction,
        detents: u32,
    },
    Pinch {
        pinch: Pinch,
        fingers: u32,
    },
    Wait(Duration),
    WaitForWindow {
        app_id: String,
        timeout: Duration,
    },
    Shot(String),
    Note(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub number: usize,
    pub source: String,
    pub step: Step,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "line {}: {}", self.line, self.message)
    }
}

impl std::error::Error for ParseError {}

pub fn parse(source: &str) -> Result<Vec<Line>, ParseError> {
    source
        .lines()
        .enumerate()
        .map(|(index, text)| (index + 1, text.trim()))
        .filter(|(_, text)| !text.is_empty() && !text.starts_with('#'))
        .map(|(number, text)| {
            parse_step(text)
                .map(|step| Line {
                    number,
                    source: text.to_owned(),
                    step,
                })
                .map_err(|message| ParseError {
                    line: number,
                    message,
                })
        })
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Quoted(String),
}

fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let mut tokens = Vec::new();
    let mut characters = text.chars().peekable();
    while let Some(&next) = characters.peek() {
        if next.is_whitespace() {
            characters.next();
        } else if next == '"' {
            characters.next();
            let mut quoted = String::new();
            loop {
                match characters.next() {
                    Some('"') => break,
                    Some('\\') => match characters.next() {
                        Some('n') => quoted.push('\n'),
                        Some('t') => quoted.push('\t'),
                        Some(escaped) => quoted.push(escaped),
                        None => return Err("unfinished escape".to_owned()),
                    },
                    Some(character) => quoted.push(character),
                    None => return Err("unclosed quote".to_owned()),
                }
            }
            tokens.push(Token::Quoted(quoted));
        } else {
            let mut word = String::new();
            while let Some(&character) = characters.peek() {
                if character.is_whitespace() || character == '"' {
                    break;
                }
                word.push(character);
                characters.next();
            }
            tokens.push(Token::Word(word));
        }
    }
    Ok(tokens)
}

struct Arguments {
    tokens: std::vec::IntoIter<Token>,
}

impl Arguments {
    fn word(&mut self, what: &str) -> Result<String, String> {
        match self.tokens.next() {
            Some(Token::Word(word)) => Ok(word),
            Some(Token::Quoted(text)) => Err(format!("expected {what}, found \"{text}\"")),
            None => Err(format!("expected {what}")),
        }
    }

    fn text(&mut self) -> Result<String, String> {
        match self.tokens.next() {
            Some(Token::Quoted(text)) => Ok(text),
            Some(Token::Word(word)) => Err(format!("expected quoted text, found {word}")),
            None => Err("expected quoted text".to_owned()),
        }
    }

    fn number(&mut self, what: &str) -> Result<u32, String> {
        let word = self.word(what)?;
        word.parse()
            .map_err(|_| format!("expected {what}, found {word}"))
    }

    fn point(&mut self) -> Result<Point, String> {
        Ok(Point {
            x: self.number("an x coordinate")?,
            y: self.number("a y coordinate")?,
        })
    }

    fn optional_point(&mut self) -> Result<Option<Point>, String> {
        if self.tokens.as_slice().is_empty() {
            return Ok(None);
        }
        self.point().map(Some)
    }

    fn peek_word(&self) -> Option<&str> {
        match self.tokens.as_slice().first() {
            Some(Token::Word(word)) => Some(word),
            _ => None,
        }
    }

    fn finish(mut self) -> Result<(), String> {
        match self.tokens.next() {
            None => Ok(()),
            Some(Token::Word(extra) | Token::Quoted(extra)) => {
                Err(format!("unexpected {extra} at the end"))
            }
        }
    }
}

fn parse_step(text: &str) -> Result<Step, String> {
    let mut arguments = Arguments {
        tokens: tokens(text)?.into_iter(),
    };
    let command = arguments.word("a command")?;
    let step = match command.as_str() {
        "key" => {
            let mut chords = vec![chord(&arguments.word("a key")?)?];
            while let Some(next) = arguments.peek_word() {
                chords.push(chord(next)?);
                arguments.tokens.next();
            }
            Step::Key(chords)
        }
        "type" => {
            let text = arguments.text()?;
            if let Some(untypeable) = text.chars().find(|&c| character_key(c).is_none()) {
                return Err(format!("cannot type {untypeable:?} on a US keyboard"));
            }
            Step::Type(text)
        }
        "move" => Step::Move(arguments.point()?),
        "click" => {
            let button = match arguments.peek_word() {
                Some("left") => Some(Button::Left),
                Some("right") => Some(Button::Right),
                Some("middle") => Some(Button::Middle),
                _ => None,
            };
            if button.is_some() {
                arguments.tokens.next();
            }
            Step::Click {
                button: button.unwrap_or(Button::Left),
                count: 1,
                at: arguments.optional_point()?,
            }
        }
        "double-click" => Step::Click {
            button: Button::Left,
            count: 2,
            at: arguments.optional_point()?,
        },
        "drag" => {
            let from = arguments.point()?;
            let joiner = arguments.word("to")?;
            if joiner != "to" {
                return Err(format!("expected to, found {joiner}"));
            }
            Step::Drag {
                from,
                to: arguments.point()?,
            }
        }
        "scroll" => {
            let direction = match arguments.word("a direction")?.as_str() {
                "up" => Direction::Up,
                "down" => Direction::Down,
                "left" => Direction::Left,
                "right" => Direction::Right,
                other => return Err(format!("unknown direction {other}")),
            };
            let detents = match arguments.peek_word() {
                Some(_) => arguments.number("a number of steps")?,
                None => 1,
            };
            Step::Scroll { direction, detents }
        }
        "pinch" => {
            let pinch = match arguments.word("in or out")?.as_str() {
                "in" => Pinch::In,
                "out" => Pinch::Out,
                other => return Err(format!("expected in or out, found {other}")),
            };
            let fingers = arguments.number("a number of fingers")?;
            if !(2..=MAX_FINGERS).contains(&fingers) {
                return Err(format!("a pinch takes 2 to {MAX_FINGERS} fingers"));
            }
            Step::Pinch { pinch, fingers }
        }
        "wait" => Step::Wait(duration(&arguments.word("a duration")?)?),
        "wait-for-window" => {
            let app_id = arguments.text()?;
            let timeout = match arguments.peek_word() {
                Some(word) => duration(word)?,
                None => DEFAULT_WINDOW_TIMEOUT,
            };
            arguments.tokens.next();
            Step::WaitForWindow { app_id, timeout }
        }
        "shot" => Step::Shot(arguments.text()?),
        "note" => Step::Note(arguments.text()?),
        other => return Err(format!("unknown command {other}")),
    };
    arguments.finish()?;
    Ok(step)
}

fn duration(word: &str) -> Result<Duration, String> {
    let invalid = || format!("expected a duration like 500ms or 2s, found {word}");
    if let Some(milliseconds) = word.strip_suffix("ms") {
        return milliseconds
            .parse()
            .map(Duration::from_millis)
            .map_err(|_| invalid());
    }
    let seconds: f64 = word
        .strip_suffix('s')
        .ok_or_else(invalid)?
        .parse()
        .map_err(|_| invalid())?;
    Duration::try_from_secs_f64(seconds).map_err(|_| invalid())
}

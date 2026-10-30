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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::{KEY_LEFTMETA, KEY_SPACE};

    fn step(text: &str) -> Result<Step, String> {
        parse_step(text)
    }

    #[test]
    fn reads_every_command() {
        let scenario = "\
# Open the terminal
key super+space
type \"terminal\\n\"

move 400 300
click right
click 10 20
double-click
drag 100 100 to 600 400
scroll down 5
pinch out 4
wait 500ms
wait 1.5s
wait-for-window \"org.unbound.Terminal\"
wait-for-window \"org.unbound.Files\" 3s
shot \"launcher open\"
note \"Nothing happens yet\"
";
        let lines = parse(scenario).unwrap();

        assert_eq!(lines.len(), 15);
        assert_eq!(lines[0].number, 2);
        assert_eq!(lines[0].source, "key super+space");
        assert_eq!(
            lines[0].step,
            Step::Key(vec![Chord {
                modifiers: vec![KEY_LEFTMETA],
                key: KEY_SPACE
            }])
        );
        assert_eq!(lines[1].step, Step::Type("terminal\n".to_owned()));
        assert_eq!(lines[2].step, Step::Move(Point { x: 400, y: 300 }));
        assert_eq!(
            lines[3].step,
            Step::Click {
                button: Button::Right,
                count: 1,
                at: None
            }
        );
        assert_eq!(
            lines[4].step,
            Step::Click {
                button: Button::Left,
                count: 1,
                at: Some(Point { x: 10, y: 20 })
            }
        );
        assert_eq!(
            lines[5].step,
            Step::Click {
                button: Button::Left,
                count: 2,
                at: None
            }
        );
        assert_eq!(
            lines[6].step,
            Step::Drag {
                from: Point { x: 100, y: 100 },
                to: Point { x: 600, y: 400 }
            }
        );
        assert_eq!(
            lines[7].step,
            Step::Scroll {
                direction: Direction::Down,
                detents: 5
            }
        );
        assert_eq!(
            lines[8].step,
            Step::Pinch {
                pinch: Pinch::Out,
                fingers: 4
            }
        );
        assert_eq!(lines[9].step, Step::Wait(Duration::from_millis(500)));
        assert_eq!(lines[10].step, Step::Wait(Duration::from_millis(1500)));
        assert_eq!(
            lines[11].step,
            Step::WaitForWindow {
                app_id: "org.unbound.Terminal".to_owned(),
                timeout: DEFAULT_WINDOW_TIMEOUT
            }
        );
        assert_eq!(
            lines[12].step,
            Step::WaitForWindow {
                app_id: "org.unbound.Files".to_owned(),
                timeout: Duration::from_secs(3)
            }
        );
        assert_eq!(lines[13].step, Step::Shot("launcher open".to_owned()));
        assert_eq!(lines[14].step, Step::Note("Nothing happens yet".to_owned()));
    }

    #[test]
    fn several_chords_on_one_line_run_in_order() {
        let Ok(Step::Key(chords)) = step("key super+tab super+tab escape") else {
            panic!("not a key step");
        };

        assert_eq!(chords.len(), 3);
        assert_eq!(chords[2].modifiers, Vec::<u16>::new());
    }

    #[test]
    fn quoted_text_keeps_escaped_quotes() {
        assert_eq!(
            step(r#"note "say \"hi\" \\ there""#),
            Ok(Step::Note(r#"say "hi" \ there"#.to_owned()))
        );
    }

    #[test]
    fn mistakes_name_their_line() {
        let error = parse("wait 1s\n\nclick 10\n").unwrap_err();

        assert_eq!(error.line, 3);
        assert_eq!(error.message, "expected a y coordinate");
    }

    #[test]
    fn rejects_malformed_steps() {
        assert!(step("wait 5").is_err());
        assert!(step("type terminal").is_err());
        assert!(step("note \"open").is_err());
        assert!(step("drag 1 2 3 4").is_err());
        assert!(step("scroll sideways").is_err());
        assert!(step("key hyper+x").is_err());
        assert!(step("shot \"a\" \"b\"").is_err());
        assert!(step("fly away").is_err());
        assert!(step("pinch sideways 4").is_err());
        assert!(step("pinch out 9").is_err());
        assert!(step("type \"café\"").is_err());
    }
}

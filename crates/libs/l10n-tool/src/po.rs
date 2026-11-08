use std::fmt::Write;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Message {
    pub context: Option<String>,
    pub english: String,
    pub plural: Option<String>,
    pub forms: Vec<String>,
    pub fuzzy: bool,
    pub comments: Vec<String>,
}

impl Message {
    pub fn translated(&self) -> bool {
        !self.fuzzy && !self.forms.is_empty() && self.forms.iter().all(|form| !form.is_empty())
    }
}

pub struct File {
    pub header: String,
    pub messages: Vec<Message>,
}

#[derive(Clone, Copy, PartialEq)]
enum Field {
    Context,
    English,
    Plural,
    Form(usize),
}

#[derive(Default)]
struct Reader {
    header: Option<String>,
    messages: Vec<Message>,
    current: Message,
    field: Option<Field>,
}

impl Reader {
    fn finish(&mut self) {
        let message = std::mem::take(&mut self.current);
        if self.field.take().is_none() {
            return;
        }
        if message.english.is_empty() && message.context.is_none() {
            self.header = Some(message.forms.into_iter().next().unwrap_or_default());
        } else {
            self.messages.push(message);
        }
    }

    fn after_translation(&self) -> bool {
        matches!(self.field, Some(Field::Form(_)))
    }
}

pub fn parse(source: &str) -> Result<File, String> {
    let mut reader = Reader::default();
    for (index, line) in source.lines().enumerate() {
        let number = index + 1;
        let line = line.trim();
        let failed = |problem: &str| format!("line {number}: {problem}");
        if line.is_empty() {
            reader.finish();
            continue;
        }
        if line.starts_with("#~") {
            continue;
        }
        if let Some(comment) = line.strip_prefix('#') {
            if reader.after_translation() {
                reader.finish();
            }
            if let Some(flags) = comment.strip_prefix(',') {
                reader.current.fuzzy |= flags.split(',').any(|flag| flag.trim() == "fuzzy");
            } else if !comment.starts_with(['.', ':', '|']) {
                let text = comment.strip_prefix(' ').unwrap_or(comment);
                reader.current.comments.push(text.to_owned());
            }
            continue;
        }
        if let Some(rest) = line.strip_prefix('"') {
            let value = unescape(rest, number)?;
            let current = &mut reader.current;
            match reader.field {
                Some(Field::Context) => current.context.get_or_insert_default().push_str(&value),
                Some(Field::English) => current.english.push_str(&value),
                Some(Field::Plural) => current.plural.get_or_insert_default().push_str(&value),
                Some(Field::Form(index)) => {
                    if let Some(form) = current.forms.get_mut(index) {
                        form.push_str(&value);
                    }
                }
                None => return Err(failed("a string outside an entry")),
            }
            continue;
        }
        let (keyword, rest) = line
            .split_once(' ')
            .ok_or_else(|| failed("expected a keyword and a string"))?;
        let quoted = rest
            .trim()
            .strip_prefix('"')
            .ok_or_else(|| failed("expected a string"))?;
        let value = unescape(quoted, number)?;
        let next = match keyword {
            "msgctxt" => Field::Context,
            "msgid" => Field::English,
            "msgid_plural" => Field::Plural,
            "msgstr" => Field::Form(0),
            other => other
                .strip_prefix("msgstr[")
                .and_then(|rest| rest.strip_suffix(']'))
                .and_then(|index| index.parse().ok())
                .map(Field::Form)
                .ok_or_else(|| failed(&format!("unknown keyword {other}")))?,
        };
        if matches!(next, Field::Context | Field::English) && reader.after_translation() {
            reader.finish();
        }
        let current = &mut reader.current;
        match next {
            Field::Context => current.context = Some(value),
            Field::English => current.english = value,
            Field::Plural => current.plural = Some(value),
            Field::Form(index) => {
                if index != current.forms.len() {
                    return Err(failed("plural forms out of order"));
                }
                current.forms.push(value);
            }
        }
        reader.field = Some(next);
    }
    reader.finish();
    Ok(File {
        header: reader.header.ok_or("no header entry (msgid \"\")")?,
        messages: reader.messages,
    })
}

fn unescape(quoted: &str, number: usize) -> Result<String, String> {
    let body = quoted
        .strip_suffix('"')
        .ok_or_else(|| format!("line {number}: unterminated string"))?;
    let mut text = String::with_capacity(body.len());
    let mut characters = body.chars();
    while let Some(character) = characters.next() {
        if character == '"' {
            return Err(format!("line {number}: an unescaped quote"));
        }
        if character != '\\' {
            text.push(character);
            continue;
        }
        text.push(match characters.next() {
            Some('n') => '\n',
            Some('t') => '\t',
            Some('"') => '"',
            Some('\\') => '\\',
            other => {
                return Err(format!(
                    "line {number}: unknown escape \\{}",
                    other.unwrap_or(' ')
                ));
            }
        });
    }
    Ok(text)
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\t', "\\t")
        .replace('\n', "\\n")
}

fn field(output: &mut String, keyword: &str, value: &str) {
    let lines = value.split_inclusive('\n').collect::<Vec<_>>();
    if lines.len() > 1 {
        let _ = writeln!(output, "{keyword} \"\"");
        for line in lines {
            let _ = writeln!(output, "\"{}\"", escape(line));
        }
    } else {
        let _ = writeln!(output, "{keyword} \"{}\"", escape(value));
    }
}

pub fn write(file: &File) -> String {
    let mut output = String::new();
    field(&mut output, "msgid", "");
    field(&mut output, "msgstr", &file.header);
    for message in &file.messages {
        output.push('\n');
        for comment in &message.comments {
            let _ = writeln!(output, "# {comment}");
        }
        if message.fuzzy {
            output.push_str("#, fuzzy\n");
        }
        if let Some(context) = &message.context {
            field(&mut output, "msgctxt", context);
        }
        field(&mut output, "msgid", &message.english);
        match &message.plural {
            Some(plural) => {
                field(&mut output, "msgid_plural", plural);
                for (index, form) in message.forms.iter().enumerate() {
                    field(&mut output, &format!("msgstr[{index}]"), form);
                }
            }
            None => field(
                &mut output,
                "msgstr",
                message.forms.first().map_or("", String::as_str),
            ),
        }
    }
    output
}

pub fn header_field<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.lines().find_map(|line| {
        let (field, value) = line.split_once(':')?;
        (field.trim() == name).then(|| value.trim())
    })
}

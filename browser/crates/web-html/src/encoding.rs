use encoding_rs::{Encoding, UTF_8, UTF_16BE, UTF_16LE, WINDOWS_1252, X_USER_DEFINED};

fn is_space(b: u8) -> bool {
    matches!(b, b'\t' | b'\n' | b'\x0C' | b'\r' | b' ')
}

pub(crate) fn label_encoding(label: &str) -> Option<&'static Encoding> {
    label_bytes_encoding(label.as_bytes())
}

fn label_bytes_encoding(label: &[u8]) -> Option<&'static Encoding> {
    let encoding = Encoding::for_label(label)?;
    Some(if encoding == UTF_16BE || encoding == UTF_16LE {
        UTF_8
    } else if encoding == X_USER_DEFINED {
        WINDOWS_1252
    } else {
        encoding
    })
}

pub(crate) fn charset_from_meta_content(content: &[u8]) -> Option<&'static Encoding> {
    let mut pos = 0;
    loop {
        let found = content
            .get(pos..)?
            .windows(7)
            .position(|w| w.eq_ignore_ascii_case(b"charset"))?;
        pos += found + 7;
        while content.get(pos).copied().is_some_and(is_space) {
            pos += 1;
        }
        if content.get(pos) == Some(&b'=') {
            pos += 1;
            break;
        }
    }
    while content.get(pos).copied().is_some_and(is_space) {
        pos += 1;
    }
    let rest = content.get(pos..)?;
    match rest.first()? {
        &quote @ (b'"' | b'\'') => {
            let value = rest.get(1..)?;
            let end = value.iter().position(|&b| b == quote)?;
            label_bytes_encoding(value.get(..end)?)
        }
        _ => {
            let end = rest
                .iter()
                .position(|&b| is_space(b) || b == b';')
                .unwrap_or(rest.len());
            label_bytes_encoding(rest.get(..end)?)
        }
    }
}

struct Prescan<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl Prescan<'_> {
    fn byte(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn starts_with(&self, prefix: &[u8]) -> bool {
        self.bytes
            .get(self.pos..)
            .is_some_and(|rest| rest.starts_with(prefix))
    }

    fn skip_to(&mut self, target: impl Fn(u8) -> bool) -> Option<()> {
        let found = self
            .bytes
            .get(self.pos..)?
            .iter()
            .position(|&b| target(b))?;
        self.pos += found;
        Some(())
    }

    fn attribute(&mut self) -> Option<Option<(Vec<u8>, Vec<u8>)>> {
        while is_space(self.byte()?) || self.byte()? == b'/' {
            self.pos += 1;
        }
        if self.byte()? == b'>' {
            return Some(None);
        }
        let mut name = Vec::new();
        let mut value = Vec::new();
        loop {
            let b = self.byte()?;
            if b == b'=' && !name.is_empty() {
                self.pos += 1;
                break;
            }
            if is_space(b) {
                while is_space(self.byte()?) {
                    self.pos += 1;
                }
                if self.byte()? != b'=' {
                    return Some(Some((name, value)));
                }
                self.pos += 1;
                break;
            }
            if b == b'/' || b == b'>' {
                return Some(Some((name, value)));
            }
            name.push(b.to_ascii_lowercase());
            self.pos += 1;
        }
        while is_space(self.byte()?) {
            self.pos += 1;
        }
        let b = self.byte()?;
        if b == b'"' || b == b'\'' {
            loop {
                self.pos += 1;
                let c = self.byte()?;
                if c == b {
                    self.pos += 1;
                    return Some(Some((name, value)));
                }
                value.push(c.to_ascii_lowercase());
            }
        }
        if b == b'>' {
            return Some(Some((name, value)));
        }
        value.push(b.to_ascii_lowercase());
        self.pos += 1;
        loop {
            let c = self.byte()?;
            if is_space(c) || c == b'>' {
                return Some(Some((name, value)));
            }
            value.push(c.to_ascii_lowercase());
            self.pos += 1;
        }
    }

    fn meta(&mut self) -> Option<Option<&'static Encoding>> {
        let mut seen: Vec<Vec<u8>> = Vec::new();
        let mut got_pragma = false;
        let mut need_pragma = None;
        let mut charset = None;
        while let Some((name, value)) = self.attribute()? {
            if seen.contains(&name) {
                continue;
            }
            match name.as_slice() {
                b"http-equiv" => got_pragma |= value == b"content-type",
                b"content" => {
                    if charset.is_none()
                        && let Some(encoding) = charset_from_meta_content(&value)
                    {
                        charset = Some(Some(encoding));
                        need_pragma = Some(true);
                    }
                }
                b"charset" => {
                    charset = Some(label_bytes_encoding(&value));
                    need_pragma = Some(false);
                }
                _ => {}
            }
            seen.push(name);
        }
        match (need_pragma, charset) {
            (Some(true), _) if !got_pragma => Some(None),
            (Some(_), Some(Some(encoding))) => Some(Some(encoding)),
            _ => Some(None),
        }
    }

    fn run(&mut self) -> Option<&'static Encoding> {
        if self.starts_with(b"<\0?\0x\0") {
            return Some(UTF_16LE);
        }
        if self.starts_with(b"\0<\0?\0x") {
            return Some(UTF_16BE);
        }
        loop {
            if self.starts_with(b"<!--") {
                self.pos += 2;
                loop {
                    self.skip_to(|b| b == b'>')?;
                    if self.pos >= 2
                        && self.bytes.get(self.pos - 2..self.pos) == Some(b"--".as_slice())
                    {
                        break;
                    }
                    self.pos += 1;
                }
            } else if self.bytes.get(self.pos..self.pos + 6).is_some_and(|w| {
                w[..5].eq_ignore_ascii_case(b"<meta") && (is_space(w[5]) || w[5] == b'/')
            }) {
                self.pos += 5;
                if let Some(encoding) = self.meta()? {
                    return Some(encoding);
                }
            } else if self.starts_with(b"<") && {
                let mut next = self.pos + 1;
                if self.bytes.get(next) == Some(&b'/') {
                    next += 1;
                }
                self.bytes.get(next).is_some_and(u8::is_ascii_alphabetic)
            } {
                self.skip_to(|b| is_space(b) || b == b'>')?;
                while self.attribute()?.is_some() {}
            } else if self.starts_with(b"<!") || self.starts_with(b"</") || self.starts_with(b"<?")
            {
                self.skip_to(|b| b == b'>')?;
            }
            self.pos += 1;
            if self.pos >= self.bytes.len() {
                return None;
            }
        }
    }
}

fn trim_controls(s: &[u8]) -> &[u8] {
    let start = s.iter().position(|&b| b > b' ').unwrap_or(s.len());
    s.get(start..).unwrap_or_default()
}

fn xml_declaration_encoding(bytes: &[u8]) -> Option<&'static Encoding> {
    if !bytes.starts_with(b"<?xml") {
        return None;
    }
    let end = bytes.iter().position(|&b| b == b'>')?;
    let declaration = bytes.get(..end)?;
    let at = declaration.windows(8).position(|w| w == b"encoding")? + 8;
    let mut rest = declaration.get(at..)?;
    rest = trim_controls(rest);
    rest = trim_controls(rest.strip_prefix(b"=")?);
    let (&quote, value) = rest.split_first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    let value = value.get(..value.iter().position(|&b| b == quote)?)?;
    if value.iter().any(|&b| b <= b' ') {
        return None;
    }
    label_bytes_encoding(value)
}

pub(crate) fn prescan(bytes: &[u8]) -> Option<&'static Encoding> {
    Prescan { bytes, pos: 0 }
        .run()
        .or_else(|| xml_declaration_encoding(bytes))
}

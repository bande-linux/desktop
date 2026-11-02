pub(crate) struct Input {
    buf: String,
    pos: usize,
    pending_cr: bool,
    pub(crate) eof: bool,
}

pub(crate) enum Lookahead {
    Match,
    Mismatch,
    NeedMore,
}

impl Input {
    pub(crate) fn new() -> Self {
        Input {
            buf: String::new(),
            pos: 0,
            pending_cr: false,
            eof: false,
        }
    }

    pub(crate) fn push(&mut self, mut text: &str) {
        if self.pos > 0 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        if self.pending_cr {
            text = text.strip_prefix('\n').unwrap_or(text);
        }
        if text.is_empty() {
            return;
        }
        self.pending_cr = text.ends_with('\r');
        let mut rest = text;
        while let Some(cr) = rest.find('\r') {
            self.buf.push_str(&rest[..cr]);
            self.buf.push('\n');
            rest = &rest[cr + 1..];
            rest = rest.strip_prefix('\n').unwrap_or(rest);
        }
        self.buf.push_str(rest);
    }

    pub(crate) fn rest(&self) -> &str {
        self.buf.get(self.pos..).unwrap_or_default()
    }

    pub(crate) fn next_char(&mut self) -> Option<char> {
        let c = self.rest().chars().next()?;
        self.pos += c.len_utf8();
        Some(c)
    }

    pub(crate) fn unread(&mut self, c: char) {
        self.pos -= c.len_utf8();
    }

    pub(crate) fn advance(&mut self, bytes: usize) {
        self.pos += bytes;
    }

    pub(crate) fn lookahead(&self, word: &str, ignore_case: bool) -> Lookahead {
        let rest = self.rest().as_bytes();
        let n = rest.len().min(word.len());
        let same = |a: &[u8], b: &[u8]| {
            if ignore_case {
                a.eq_ignore_ascii_case(b)
            } else {
                a == b
            }
        };
        if !same(&rest[..n], &word.as_bytes()[..n]) {
            Lookahead::Mismatch
        } else if n == word.len() {
            Lookahead::Match
        } else if self.eof {
            Lookahead::Mismatch
        } else {
            Lookahead::NeedMore
        }
    }
}

use encoding_rs::Encoding;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentType {
    pub essence: String,
    pub charset: Option<&'static Encoding>,
}

impl ContentType {
    pub fn parse(value: &[u8]) -> Option<Self> {
        let value = trim(value);
        let slash = value.iter().position(|&byte| byte == b'/')?;
        let kind = &value[..slash];
        let rest = &value[slash + 1..];
        let end = rest
            .iter()
            .position(|&byte| byte == b';')
            .unwrap_or(rest.len());
        let subtype = trim_end(&rest[..end]);
        if !is_token(kind) || !is_token(subtype) {
            return None;
        }
        let essence = format!(
            "{}/{}",
            String::from_utf8_lossy(kind).to_ascii_lowercase(),
            String::from_utf8_lossy(subtype).to_ascii_lowercase()
        );
        Some(Self {
            essence,
            charset: charset(&rest[end..]),
        })
    }

    pub fn is_html(&self) -> bool {
        self.essence == "text/html"
    }
}

fn charset(mut parameters: &[u8]) -> Option<&'static Encoding> {
    while let Some((&b';', rest)) = parameters.split_first() {
        let rest = trim_start(rest);
        let name_end = rest
            .iter()
            .position(|&byte| byte == b';' || byte == b'=')
            .unwrap_or(rest.len());
        let name = &rest[..name_end];
        let rest = &rest[name_end..];
        let Some((&b'=', rest)) = rest.split_first() else {
            parameters = rest;
            continue;
        };
        let (value, rest) = match rest.split_first() {
            Some((&b'"', quoted)) => quoted_string(quoted),
            _ => {
                let end = rest
                    .iter()
                    .position(|&byte| byte == b';')
                    .unwrap_or(rest.len());
                (trim_end(&rest[..end]).to_vec(), &rest[end..])
            }
        };
        if name.eq_ignore_ascii_case(b"charset") && !value.is_empty() {
            return Encoding::for_label(&value);
        }
        parameters = rest;
    }
    None
}

fn quoted_string(mut input: &[u8]) -> (Vec<u8>, &[u8]) {
    let mut value = Vec::new();
    while let Some((&byte, rest)) = input.split_first() {
        input = rest;
        match byte {
            b'"' => break,
            b'\\' => {
                if let Some((&escaped, rest)) = input.split_first() {
                    value.push(escaped);
                    input = rest;
                }
            }
            byte => value.push(byte),
        }
    }
    let end = input
        .iter()
        .position(|&byte| byte == b';')
        .unwrap_or(input.len());
    (value, &input[end..])
}

fn is_token(bytes: &[u8]) -> bool {
    !bytes.is_empty()
        && bytes
            .iter()
            .all(|&byte| byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte))
}

fn is_http_whitespace(byte: &u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r')
}

fn trim_start(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|byte| !is_http_whitespace(byte))
        .unwrap_or(bytes.len());
    &bytes[start..]
}

fn trim_end(bytes: &[u8]) -> &[u8] {
    let end = bytes
        .iter()
        .rposition(|byte| !is_http_whitespace(byte))
        .map_or(0, |end| end + 1);
    &bytes[..end]
}

fn trim(bytes: &[u8]) -> &[u8] {
    trim_end(trim_start(bytes))
}

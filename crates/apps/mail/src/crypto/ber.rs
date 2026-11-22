const MAX_DEPTH: usize = 24;
const MAX_INPUT: usize = 64 << 20;
const OCTET_STRING: u8 = 0x04;
const CONSTRUCTED_OCTET_STRING: u8 = 0x24;
const OID: u8 = 0x06;
const SEQUENCE: u8 = 0x30;
const CONSTRUCTED: u8 = 0x20;
const ENCRYPTED_CONTENT: u8 = 0xa0;

struct Element {
    tag: Vec<u8>,
    content: Vec<u8>,
}

fn take<'a>(input: &mut &'a [u8], count: usize) -> Option<&'a [u8]> {
    let (taken, rest) = input.split_at_checked(count)?;
    *input = rest;
    Some(taken)
}

fn identifier(input: &mut &[u8]) -> Option<Vec<u8>> {
    let first = *take(input, 1)?.first()?;
    let mut tag = vec![first];
    if first & 0x1f == 0x1f {
        loop {
            let byte = *take(input, 1)?.first()?;
            tag.push(byte);
            if tag.len() > 5 {
                return None;
            }
            if byte & 0x80 == 0 {
                break;
            }
        }
    }
    Some(tag)
}

fn length(input: &mut &[u8]) -> Option<Option<usize>> {
    let first = *take(input, 1)?.first()?;
    match first {
        0x80 => Some(None),
        0x00..=0x7f => Some(Some(usize::from(first))),
        _ => {
            let count = usize::from(first & 0x7f);
            if count > 4 {
                return None;
            }
            let bytes = take(input, count)?;
            let length = bytes
                .iter()
                .fold(0usize, |length, byte| (length << 8) | usize::from(*byte));
            Some(Some(length))
        }
    }
}

fn encode_length(length: usize, out: &mut Vec<u8>) {
    if length < 0x80 {
        out.push(length as u8);
        return;
    }
    let bytes = length.to_be_bytes();
    let skip = bytes.iter().take_while(|byte| **byte == 0).count();
    out.push(0x80 | (bytes.len() - skip) as u8);
    out.extend_from_slice(&bytes[skip..]);
}

fn write(element: &Element, out: &mut Vec<u8>) {
    out.extend_from_slice(&element.tag);
    encode_length(element.content.len(), out);
    out.extend_from_slice(&element.content);
}

fn joined_strings(children: &[Element]) -> Option<Vec<u8>> {
    children
        .iter()
        .all(|child| child.tag == [OCTET_STRING])
        .then(|| {
            children
                .iter()
                .flat_map(|child| child.content.clone())
                .collect()
        })
}

fn element(input: &mut &[u8], depth: usize) -> Option<Element> {
    if depth > MAX_DEPTH {
        return None;
    }
    let tag = identifier(input)?;
    let length = length(input)?;
    if tag[0] & CONSTRUCTED == 0 {
        let content = take(input, length?)?.to_vec();
        return Some(Element { tag, content });
    }
    let mut children = Vec::new();
    match length {
        Some(length) => {
            let mut inner = take(input, length)?;
            while !inner.is_empty() {
                children.push(element(&mut inner, depth + 1)?);
            }
        }
        None => loop {
            if input.starts_with(&[0, 0]) {
                *input = &input[2..];
                break;
            }
            if input.is_empty() {
                return None;
            }
            children.push(element(input, depth + 1)?);
        },
    }
    normalize_children(&tag, &mut children);
    if tag == [CONSTRUCTED_OCTET_STRING] {
        return Some(Element {
            tag: vec![OCTET_STRING],
            content: joined_strings(&children)?,
        });
    }
    let mut content = Vec::new();
    for child in &children {
        write(child, &mut content);
    }
    Some(Element { tag, content })
}

fn normalize_children(tag: &[u8], children: &mut [Element]) {
    let encrypted_content_info = tag == [SEQUENCE]
        && children.len() == 3
        && children[0].tag == [OID]
        && children[1].tag == [SEQUENCE]
        && children[2].tag == [ENCRYPTED_CONTENT];
    if !encrypted_content_info {
        return;
    }
    let mut inner = children[2].content.as_slice();
    let mut chunks = Vec::new();
    while !inner.is_empty() {
        match element(&mut inner, MAX_DEPTH) {
            Some(chunk) => chunks.push(chunk),
            None => return,
        }
    }
    if let Some(joined) = joined_strings(&chunks) {
        children[2] = Element {
            tag: vec![ENCRYPTED_CONTENT & !CONSTRUCTED],
            content: joined,
        };
    }
}

pub fn to_der(input: &[u8]) -> Option<Vec<u8>> {
    if input.len() > MAX_INPUT {
        return None;
    }
    let mut rest = input;
    let element = element(&mut rest, 0)?;
    if rest.iter().any(|byte| *byte != 0) {
        return None;
    }
    let mut out = Vec::with_capacity(input.len());
    write(&element, &mut out);
    Some(out)
}

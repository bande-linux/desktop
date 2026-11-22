use std::io::{BufRead, Read};

use crate::ipp::{Error, Result};

const LONGEST_LINE: u64 = 8 << 10;
const MOST_HEADERS: usize = 100;

/// An HTTP/1.1 answer from a device, read within limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub location: Option<String>,
    pub body: Vec<u8>,
}

fn read_line(reader: &mut impl BufRead, line: &mut String) -> Result<usize> {
    line.clear();
    let read = reader.by_ref().take(LONGEST_LINE).read_line(line)?;
    if read > 0 && !line.ends_with('\n') {
        return Err(Error::Malformed);
    }
    Ok(read)
}

fn read_exactly(reader: &mut impl BufRead, body: &mut Vec<u8>, count: usize) -> Result<()> {
    let start = body.len();
    reader.by_ref().take(count as u64).read_to_end(body)?;
    match body.len() - start == count {
        true => Ok(()),
        false => Err(Error::Malformed),
    }
}

/// Reads an HTTP/1.1 answer whose body is at most `most` bytes.
pub fn read_response(mut reader: impl BufRead, most: usize) -> Result<Response> {
    let mut line = String::new();
    read_line(&mut reader, &mut line)?;
    let status: u16 = line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .ok_or(Error::Malformed)?;
    let mut length = None;
    let mut chunked = false;
    let mut location = None;
    for _ in 0..=MOST_HEADERS {
        if read_line(&mut reader, &mut line)? == 0 {
            return Err(Error::Malformed);
        }
        let header = line.trim_end();
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            length = Some(value.parse::<usize>().map_err(|_| Error::Malformed)?);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            chunked = value.eq_ignore_ascii_case("chunked");
        } else if name.eq_ignore_ascii_case("location") {
            location = Some(value.to_owned());
        }
    }
    if !line.trim_end().is_empty() {
        return Err(Error::Malformed);
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            read_line(&mut reader, &mut line)?;
            let size = usize::from_str_radix(line.trim().split(';').next().unwrap_or(""), 16)
                .map_err(|_| Error::Malformed)?;
            if size == 0 {
                break;
            }
            if size > most.saturating_sub(body.len()) {
                return Err(Error::Malformed);
            }
            read_exactly(&mut reader, &mut body, size)?;
            read_line(&mut reader, &mut line)?;
        }
    } else if let Some(length) = length {
        if length > most {
            return Err(Error::Malformed);
        }
        read_exactly(&mut reader, &mut body, length)?;
    } else {
        reader.take(most as u64 + 1).read_to_end(&mut body)?;
        if body.len() > most {
            return Err(Error::Malformed);
        }
    }
    Ok(Response {
        status,
        location,
        body,
    })
}

use std::io::Read;

use crate::error::{Error, Result};
use crate::net::Network;

const MAX_REDIRECTS: u32 = 3;

pub const XML: &str = "application/xml, text/xml";
pub const IMAGES: &str = "image/*";

pub fn get(url: &str, accept: &str, network: &Network, limit: usize) -> Result<Vec<u8>> {
    let agent: bande_https::ureq::Agent = bande_https::config("mail")
        .https_only(true)
        .max_redirects(MAX_REDIRECTS)
        .timeout_global(Some(network.timeout))
        .build()
        .into();
    let response = agent
        .get(url)
        .header("Accept", accept)
        .call()
        .map_err(|error| match error {
            bande_https::ureq::Error::StatusCode(status) => {
                Error::Refused(format!("HTTP {status}"))
            }
            error if bande_https::untrusted(&error) => Error::Tls(error.to_string()),
            error => Error::Io(error.into_io()),
        })?;
    let mut body = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(limit as u64 + 1)
        .read_to_end(&mut body)?;
    match body.len() > limit {
        true => Err(Error::Protocol(
            "an HTTP answer that is too large".to_owned(),
        )),
        false => Ok(body),
    }
}

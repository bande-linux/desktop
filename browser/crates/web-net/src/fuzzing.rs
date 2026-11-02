use hyper::HeaderMap;
use hyper::header::{CACHE_CONTROL, DATE, ETAG, EXPIRES, HeaderValue, LAST_MODIFIED, VARY};
use url::Url;

use crate::cache::{Cache, Lookup};
use crate::cookies::{Context, Jar};
use crate::hsts::Hsts;
use crate::site::Site;
use crate::{ContentType, Response};

const NOW: u64 = 1_800_000_000;

fn page() -> Option<Url> {
    Url::parse("https://www.example.co.uk/a/b/page.html").ok()
}

pub fn set_cookie(text: &str) {
    let Some(url) = page() else {
        return;
    };
    let Some(site) = Site::of(&url) else {
        return;
    };
    let mut jar = Jar::default();
    for (index, header) in text.split('\n').enumerate() {
        let context = Context {
            top_level: &site,
            first_party: index % 3 != 0,
            same_site: index % 2 == 0,
            navigation: index % 5 == 0,
        };
        jar.store(&url, header, context, NOW);
        let _ = jar.header(&url, context, NOW);
    }
    let reloaded = Jar::load(&jar.save(), NOW);
    let _ = reloaded.save();
}

pub fn cookie_date(text: &str) {
    let _ = crate::date::parse_cookie_date(text);
}

pub fn cache_headers(text: &str) {
    let Some(url) = page() else {
        return;
    };
    let mut headers = HeaderMap::new();
    let names = [CACHE_CONTROL, VARY, EXPIRES, DATE, LAST_MODIFIED, ETAG];
    for (line, name) in text.split('\n').zip(names.iter().cycle()) {
        if let Ok(value) = HeaderValue::from_str(line) {
            headers.append(name.clone(), value);
        }
    }
    let response = Response {
        url: url.clone(),
        status: 200,
        content_type: ContentType::parse(b"text/html"),
        body: text.as_bytes().to_vec(),
    };
    let mut cache = Cache::memory();
    cache.store(
        "https://example.co.uk",
        &response,
        &headers,
        text.len() % 2 == 0,
        NOW,
    );
    for now in [NOW, NOW + 1, NOW + 86_400 * 365] {
        if let Lookup::Stale { .. } = cache.lookup("https://example.co.uk", &url, now) {
            let _ = cache.revalidated("https://example.co.uk", &url, &headers, now);
        }
    }
}

pub fn hsts(text: &str) {
    let mut hsts = Hsts::default();
    for line in text.split('\n') {
        if let Ok(url) = Url::parse("https://sub.example.org/") {
            hsts.observe(&url, line, NOW);
        }
    }
    if let Ok(mut url) = Url::parse("http://a.sub.example.org:80/x") {
        hsts.upgrade(&mut url, NOW);
    }
    let _ = Hsts::load(&hsts.save(), NOW).save();
}

pub fn public_suffix_list(text: &str) {
    let hosts: Vec<&str> = text
        .lines()
        .rev()
        .take(8)
        .chain(["www.example.co.uk", "a.b.c.d", "", "."])
        .collect();
    crate::site::fuzz_rules(text, &hosts);
}

pub fn data_url(text: &str) {
    if let Ok(url) = Url::parse(&format!("data:{text}")) {
        let _ = crate::data::fetch(url);
    }
}

pub fn hsts_preload(data: &[u8]) {
    let split = data
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(data.len());
    let (table, name) = data.split_at(split);
    let name = String::from_utf8_lossy(name);
    let _ = crate::hsts::preloaded(table, &name);
    let _ = crate::hsts::preloaded(table, "example.org");
}

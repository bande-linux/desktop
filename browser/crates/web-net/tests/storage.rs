use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use web_net::{FetchId, Fetcher, Request, Response, Storage, Url};

fn serve(listener: TcpListener, hits: Arc<AtomicUsize>) {
    for stream in listener.incoming() {
        let Ok(mut stream) = stream else {
            return;
        };
        let mut reader = BufReader::new(&stream);
        let mut request_line = String::new();
        if reader.read_line(&mut request_line).is_err() {
            continue;
        }
        let mut cookie = String::new();
        let mut conditional = false;
        let mut line = String::new();
        while reader.read_line(&mut line).is_ok_and(|read| read > 2) {
            let lower = line.to_ascii_lowercase();
            if let Some(value) = lower.strip_prefix("cookie:") {
                cookie = value.trim().to_owned();
            }
            conditional |= lower.starts_with("if-none-match:");
            line.clear();
        }
        hits.fetch_add(1, Ordering::SeqCst);
        let path = request_line.split(' ').nth(1).unwrap_or_default();
        let (head, body) = match path {
            "/set" => (
                "200 OK\r\nSet-Cookie: kept=1; Max-Age=3600; Path=/\r\nSet-Cookie: session=2; SameSite=Strict\r\nCache-Control: max-age=60",
                "set".to_owned(),
            ),
            "/echo" => ("200 OK\r\nCache-Control: no-store", cookie),
            _ if conditional => ("304 Not Modified\r\nETag: \"v1\"", String::new()),
            _ => (
                "200 OK\r\nETag: \"v1\"\r\nCache-Control: no-cache",
                "tagged".to_owned(),
            ),
        };
        let _ = write!(
            stream,
            "HTTP/1.1 {head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    }
}

fn wait(fetcher: &mut Fetcher, id: FetchId) -> Result<Response, Box<dyn std::error::Error>> {
    loop {
        let fd = fetcher.fd();
        rustix::event::poll(
            &mut [rustix::event::PollFd::new(
                &fd,
                rustix::event::PollFlags::IN,
            )],
            None,
        )?;
        for (done, result) in fetcher.completed() {
            if done == id {
                return Ok(result?);
            }
        }
    }
}

fn get(fetcher: &mut Fetcher, request: Request) -> Result<String, Box<dyn std::error::Error>> {
    let id = fetcher.request(request);
    Ok(String::from_utf8(wait(fetcher, id)?.body)?)
}

#[test]
fn cookies_cache_and_storage() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let port = listener.local_addr()?.port();
    let hits = Arc::new(AtomicUsize::new(0));
    let served = Arc::clone(&hits);
    std::thread::spawn(move || serve(listener, served));
    let folder = std::env::temp_dir().join(format!("web-net-storage-{}", std::process::id()));
    let local = |path: &str| Url::parse(&format!("http://localhost:{port}{path}"));
    let other = |path: &str| Url::parse(&format!("http://127.0.0.1:{port}{path}"));
    let navigation = |url: Url| Request::navigation(url, None);

    let mut fetcher = Fetcher::with_storage(Storage::Disk(folder.clone()))?;
    assert_eq!(get(&mut fetcher, navigation(local("/set")?))?, "set");
    assert_eq!(get(&mut fetcher, navigation(local("/set")?))?, "set");
    assert_eq!(hits.load(Ordering::SeqCst), 1);
    assert_eq!(
        get(&mut fetcher, navigation(local("/echo")?))?,
        "kept=1; session=2"
    );
    let cross_site = Request::navigation(local("/echo")?, Some(other("/")?));
    assert_eq!(get(&mut fetcher, cross_site)?, "kept=1");
    let third_party = Request::subresource(local("/echo")?, &other("/")?);
    assert_eq!(get(&mut fetcher, third_party)?, "");
    assert_eq!(get(&mut fetcher, navigation(local("/tagged")?))?, "tagged");
    assert_eq!(get(&mut fetcher, navigation(local("/tagged")?))?, "tagged");
    drop(fetcher);

    let mut restarted = Fetcher::with_storage(Storage::Disk(folder.clone()))?;
    assert_eq!(get(&mut restarted, navigation(local("/echo")?))?, "kept=1");
    let before = hits.load(Ordering::SeqCst);
    assert_eq!(get(&mut restarted, navigation(local("/set")?))?, "set");
    assert_eq!(hits.load(Ordering::SeqCst), before);
    drop(restarted);

    let mut private = Fetcher::new()?;
    assert_eq!(get(&mut private, navigation(local("/echo")?))?, "");
    std::fs::remove_dir_all(&folder)?;
    Ok(())
}

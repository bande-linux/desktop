use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

use flate2::Compression;
use flate2::write::GzEncoder;
use web_net::{FetchError, Fetcher, MAX_BODY, Url};

fn gzip(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::fast());
    encoder.write_all(bytes)?;
    encoder.finish()
}

fn serve(listener: TcpListener) -> std::io::Result<()> {
    let page = gzip("<p>caf\u{e9}</p>".as_bytes())?;
    let bomb = gzip(&vec![0; MAX_BODY + 1])?;
    for stream in listener.incoming().take(3) {
        let mut stream = stream?;
        let mut request_line = String::new();
        let mut reader = BufReader::new(&stream);
        reader.read_line(&mut request_line)?;
        while reader.read_line(&mut String::new())? > 2 {}
        let (head, body): (String, &[u8]) = match request_line.split(' ').nth(1) {
            Some("/start") => ("302 Found\r\nLocation: /page#top".into(), &[]),
            Some("/page") => (
                "200 OK\r\nContent-Type: text/html; charset=\"utf-8\"\r\nContent-Encoding: gzip"
                    .into(),
                &page,
            ),
            _ => ("200 OK\r\nContent-Encoding: gzip".into(), &bomb),
        };
        write!(
            stream,
            "HTTP/1.1 {head}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )?;
        let _ = stream.write_all(body);
    }
    Ok(())
}

#[test]
fn follows_redirects_decodes_and_bounds_bodies() -> Result<(), Box<dyn std::error::Error>> {
    let listener = TcpListener::bind("127.0.0.1:0")?;
    let origin = format!("http://{}", listener.local_addr()?);
    let server = std::thread::spawn(move || serve(listener));
    let mut fetcher = Fetcher::new()?;
    let page = fetcher.fetch(Url::parse(&format!("{origin}/start"))?);
    let bomb = fetcher.fetch(Url::parse(&format!("{origin}/bomb"))?);
    let mut completions = Vec::new();
    while completions.len() < 2 {
        let fd = fetcher.fd();
        rustix::event::poll(
            &mut [rustix::event::PollFd::new(
                &fd,
                rustix::event::PollFlags::IN,
            )],
            None,
        )?;
        completions.extend(fetcher.completed());
    }
    for (id, result) in completions {
        if id == page {
            let response = result?;
            assert_eq!(response.url.as_str(), format!("{origin}/page#top"));
            assert!(
                response
                    .content_type
                    .as_ref()
                    .is_some_and(|content_type| content_type.is_html())
            );
            assert_eq!(response.text(), "<p>caf\u{e9}</p>");
        } else {
            assert_eq!(id, bomb);
            assert!(matches!(result, Err(FetchError::TooLarge)));
        }
    }
    server.join().map_err(|_| "the server panicked")??;
    Ok(())
}

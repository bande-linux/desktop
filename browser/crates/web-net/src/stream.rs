use std::future::Future;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Arc, Mutex, PoisonError};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use hyper::rt::ReadBufCursor;
use rustls::ClientConnection;
use url::Host;

use crate::reactor::{Reactor, Socket};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const READ_CHUNK: usize = 16 * 1024;

#[derive(Default)]
struct DialState {
    result: Option<io::Result<TcpStream>>,
    waker: Option<Waker>,
}

pub(crate) struct Dial(Arc<Mutex<DialState>>);

pub(crate) fn dial(host: Host<String>, port: u16) -> Dial {
    let state = Arc::new(Mutex::new(DialState::default()));
    let shared = Arc::clone(&state);
    let spawned = std::thread::Builder::new()
        .name("web-net-dial".into())
        .spawn(move || {
            let result = connect(&host, port);
            let mut state = shared.lock().unwrap_or_else(PoisonError::into_inner);
            state.result = Some(result);
            if let Some(waker) = state.waker.take() {
                waker.wake();
            }
        });
    if let Err(error) = spawned {
        state.lock().unwrap_or_else(PoisonError::into_inner).result = Some(Err(error));
    }
    Dial(state)
}

fn connect(host: &Host<String>, port: u16) -> io::Result<TcpStream> {
    let addresses: Vec<SocketAddr> = match host {
        Host::Domain(domain) => (domain.as_str(), port).to_socket_addrs()?.collect(),
        Host::Ipv4(address) => vec![SocketAddr::from((*address, port))],
        Host::Ipv6(address) => vec![SocketAddr::from((*address, port))],
    };
    let mut last = io::Error::new(io::ErrorKind::NotFound, "the host has no addresses");
    for address in addresses {
        match TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
            Ok(stream) => {
                stream.set_nodelay(true)?;
                return Ok(stream);
            }
            Err(error) => last = error,
        }
    }
    Err(last)
}

impl Future for Dial {
    type Output = io::Result<TcpStream>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut state = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        match state.result.take() {
            Some(result) => Poll::Ready(result),
            None => {
                state.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        }
    }
}

pub(crate) enum Stream {
    Plain(Socket),
    Tls(Box<Tls>),
}

pub(crate) struct Tls {
    socket: Socket,
    session: ClientConnection,
    closed: bool,
}

impl Stream {
    pub(crate) fn plain(stream: TcpStream, reactor: &Rc<Reactor>) -> io::Result<Self> {
        Ok(Self::Plain(Socket::new(stream, reactor)?))
    }

    pub(crate) fn tls(
        stream: TcpStream,
        reactor: &Rc<Reactor>,
        session: ClientConnection,
    ) -> io::Result<Self> {
        Ok(Self::Tls(Box::new(Tls {
            socket: Socket::new(stream, reactor)?,
            session,
            closed: false,
        })))
    }
}

impl Tls {
    fn poll_send(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while self.session.wants_write() {
            let Self {
                socket, session, ..
            } = self;
            match socket.poll_write_with(cx, |mut stream| session.write_tls(&mut stream)) {
                Poll::Ready(Ok(0)) => return Poll::Ready(Err(io::ErrorKind::WriteZero.into())),
                Poll::Ready(Ok(_)) => {}
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Pending => return Poll::Pending,
            }
        }
        Poll::Ready(Ok(()))
    }

    fn poll_read(&mut self, cx: &mut Context<'_>, buffer: &mut [u8]) -> Poll<io::Result<usize>> {
        loop {
            if let Poll::Ready(Err(error)) = self.poll_send(cx) {
                return Poll::Ready(Err(error));
            }
            match self.session.reader().read(buffer) {
                Ok(read) => return Poll::Ready(Ok(read)),
                Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                    return Poll::Ready(Ok(0));
                }
                Err(error) if error.kind() != io::ErrorKind::WouldBlock => {
                    return Poll::Ready(Err(error));
                }
                Err(_) if self.closed => return Poll::Ready(Ok(0)),
                Err(_) => {}
            }
            let Self {
                socket, session, ..
            } = self;
            match socket.poll_read_with(cx, |mut stream| session.read_tls(&mut stream)) {
                Poll::Ready(Ok(0)) => self.closed = true,
                Poll::Ready(Ok(_)) => {}
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Pending => return Poll::Pending,
            }
            if let Err(error) = self.session.process_new_packets() {
                let _ = self.poll_send(cx);
                return Poll::Ready(Err(io::Error::new(io::ErrorKind::InvalidData, error)));
            }
        }
    }

    fn poll_write(&mut self, cx: &mut Context<'_>, buffer: &[u8]) -> Poll<io::Result<usize>> {
        if buffer.is_empty() {
            return Poll::Ready(Ok(0));
        }
        loop {
            let written = self.session.writer().write(buffer)?;
            let sent = self.poll_send(cx);
            if written > 0 {
                return match sent {
                    Poll::Ready(Err(error)) => Poll::Ready(Err(error)),
                    _ => Poll::Ready(Ok(written)),
                };
            }
            match sent {
                Poll::Ready(Ok(())) => {}
                Poll::Ready(Err(error)) => return Poll::Ready(Err(error)),
                Poll::Pending => return Poll::Pending,
            }
        }
    }

    fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.session.send_close_notify();
        match self.poll_send(cx) {
            Poll::Ready(Ok(())) => Poll::Ready(self.socket.shutdown()),
            other => other,
        }
    }
}

impl hyper::rt::Read for Stream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut cursor: ReadBufCursor<'_>,
    ) -> Poll<io::Result<()>> {
        let mut chunk = [0; READ_CHUNK];
        let room = cursor.remaining().min(READ_CHUNK);
        let buffer = &mut chunk[..room];
        let read = match self.get_mut() {
            Self::Plain(socket) => socket.poll_read_with(cx, |mut stream| stream.read(buffer)),
            Self::Tls(tls) => tls.poll_read(cx, buffer),
        };
        read.map_ok(|read| cursor.put_slice(&chunk[..read]))
    }
}

impl hyper::rt::Write for Stream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            Self::Plain(socket) => socket.poll_write_with(cx, |mut stream| stream.write(buffer)),
            Self::Tls(tls) => tls.poll_write(cx, buffer),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(_) => Poll::Ready(Ok(())),
            Self::Tls(tls) => tls.poll_send(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            Self::Plain(socket) => Poll::Ready(socket.shutdown()),
            Self::Tls(tls) => tls.poll_shutdown(cx),
        }
    }
}

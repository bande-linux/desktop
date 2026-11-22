use std::io::{self, Read, Write};
use std::net::TcpStream;

use rustls::{ClientConnection, ServerConnection, StreamOwned};

/// An RFB connection: plain TCP, or TLS after VeNCrypt on either side.
pub enum Transport {
    Plain(TcpStream),
    Server(Box<StreamOwned<ServerConnection, TcpStream>>),
    Client(Box<StreamOwned<ClientConnection, TcpStream>>),
}

impl Transport {
    pub fn socket(&self) -> &TcpStream {
        match self {
            Self::Plain(socket) => socket,
            Self::Server(stream) => &stream.sock,
            Self::Client(stream) => &stream.sock,
        }
    }

    /// Whether TLS holds decrypted bytes that waiting on the socket would not see.
    pub fn buffered(&mut self) -> bool {
        let state = match self {
            Self::Plain(_) => return false,
            Self::Server(stream) => stream.conn.process_new_packets(),
            Self::Client(stream) => stream.conn.process_new_packets(),
        };
        state.is_ok_and(|state| state.plaintext_bytes_to_read() > 0)
    }

    pub fn is_encrypted(&self) -> bool {
        !matches!(self, Self::Plain(_))
    }
}

impl Read for Transport {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Plain(socket) => socket.read(buffer),
            Self::Server(stream) => stream.read(buffer),
            Self::Client(stream) => stream.read(buffer),
        }
    }
}

impl Write for Transport {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        match self {
            Self::Plain(socket) => socket.write(buffer),
            Self::Server(stream) => stream.write(buffer),
            Self::Client(stream) => stream.write(buffer),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Plain(socket) => socket.flush(),
            Self::Server(stream) => stream.flush(),
            Self::Client(stream) => stream.flush(),
        }
    }
}

pub fn read_array<const N: usize>(from: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    from.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub fn read_u32(from: &mut impl Read) -> io::Result<u32> {
    read_array::<4>(from).map(u32::from_be_bytes)
}

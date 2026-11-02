use std::fmt;

use url::Url;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Malformed;

impl fmt::Display for Malformed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a malformed message")
    }
}

impl std::error::Error for Malformed {}

impl From<Malformed> for std::io::Error {
    fn from(error: Malformed) -> Self {
        std::io::Error::new(std::io::ErrorKind::InvalidData, error)
    }
}

#[derive(Default)]
pub struct Writer {
    pub(crate) bytes: Vec<u8>,
}

impl Writer {
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    pub fn put<T: Wire>(&mut self, value: &T) {
        value.write(self);
    }

    fn raw(&mut self, bytes: &[u8]) {
        self.bytes.extend_from_slice(bytes);
    }
}

pub struct Reader<'a> {
    bytes: &'a [u8],
}

impl<'a> Reader<'a> {
    pub fn new(bytes: &'a [u8]) -> Self {
        Self { bytes }
    }

    pub fn get<T: Wire>(&mut self) -> Result<T, Malformed> {
        T::read(self)
    }

    pub fn finish(self) -> Result<(), Malformed> {
        self.bytes.is_empty().then_some(()).ok_or(Malformed)
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], Malformed> {
        let (taken, rest) = self.bytes.split_at_checked(count).ok_or(Malformed)?;
        self.bytes = rest;
        Ok(taken)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Malformed> {
        self.take(N)?.try_into().map_err(|_| Malformed)
    }

    fn count(&mut self) -> Result<usize, Malformed> {
        let count = u32::read(self)? as usize;
        (count <= self.bytes.len())
            .then_some(count)
            .ok_or(Malformed)
    }
}

pub trait Wire: Sized {
    fn write(&self, writer: &mut Writer);
    fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed>;
}

macro_rules! numbers {
    ($($ty:ty),*) => {$(
        impl Wire for $ty {
            fn write(&self, writer: &mut Writer) {
                writer.raw(&self.to_le_bytes());
            }

            fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed> {
                reader.array().map(<$ty>::from_le_bytes)
            }
        }
    )*};
}

numbers!(u8, u16, u32, u64);

impl Wire for f32 {
    fn write(&self, writer: &mut Writer) {
        writer.raw(&self.to_le_bytes());
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed> {
        let value = f32::from_le_bytes(reader.array()?);
        (value.is_finite() && value.abs() <= MAX_COORDINATE)
            .then_some(value)
            .ok_or(Malformed)
    }
}

pub const MAX_COORDINATE: f32 = 1.0e8;

impl Wire for bool {
    fn write(&self, writer: &mut Writer) {
        u8::from(*self).write(writer);
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed> {
        match u8::read(reader)? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Malformed),
        }
    }
}

impl Wire for String {
    fn write(&self, writer: &mut Writer) {
        (self.len() as u32).write(writer);
        writer.raw(self.as_bytes());
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed> {
        let count = reader.count()?;
        let bytes = reader.take(count)?;
        std::str::from_utf8(bytes)
            .map(str::to_owned)
            .map_err(|_| Malformed)
    }
}

impl Wire for Url {
    fn write(&self, writer: &mut Writer) {
        self.as_str().to_owned().write(writer);
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed> {
        Url::parse(&String::read(reader)?).map_err(|_| Malformed)
    }
}

impl<T: Wire> Wire for Option<T> {
    fn write(&self, writer: &mut Writer) {
        match self {
            None => 0u8.write(writer),
            Some(value) => {
                1u8.write(writer);
                value.write(writer);
            }
        }
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed> {
        match bool::read(reader)? {
            false => Ok(None),
            true => T::read(reader).map(Some),
        }
    }
}

impl<T: Wire> Wire for Vec<T> {
    fn write(&self, writer: &mut Writer) {
        (self.len() as u32).write(writer);
        for value in self {
            value.write(writer);
        }
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed> {
        let count = reader.count()?;
        (0..count).map(|_| T::read(reader)).collect()
    }
}

impl<A: Wire, B: Wire> Wire for (A, B) {
    fn write(&self, writer: &mut Writer) {
        self.0.write(writer);
        self.1.write(writer);
    }

    fn read(reader: &mut Reader<'_>) -> Result<Self, Malformed> {
        Ok((A::read(reader)?, B::read(reader)?))
    }
}

macro_rules! wire_struct {
    ($(#[$meta:meta])* pub struct $name:ident { $(pub $field:ident : $ty:ty),* $(,)? }) => {
        $(#[$meta])*
        pub struct $name { $(pub $field: $ty),* }

        impl $crate::wire::Wire for $name {
            fn write(&self, writer: &mut $crate::wire::Writer) {
                $(writer.put(&self.$field);)*
            }

            fn read(reader: &mut $crate::wire::Reader<'_>) -> Result<Self, $crate::wire::Malformed> {
                Ok(Self { $($field: reader.get()?),* })
            }
        }
    };
}

macro_rules! wire_enum {
    ($(#[$meta:meta])* pub enum $name:ident {
        $($variant:ident $({ $($field:ident : $ty:ty),* $(,)? })? = $tag:literal),* $(,)?
    }) => {
        $(#[$meta])*
        pub enum $name { $($variant $({ $($field: $ty),* })?),* }

        impl $crate::wire::Wire for $name {
            fn write(&self, writer: &mut $crate::wire::Writer) {
                match self {
                    $(Self::$variant $({ $($field),* })? => {
                        writer.put(&($tag as u8));
                        $($(writer.put($field);)*)?
                    })*
                }
            }

            fn read(reader: &mut $crate::wire::Reader<'_>) -> Result<Self, $crate::wire::Malformed> {
                match reader.get::<u8>()? {
                    $($tag => Ok(Self::$variant $({ $($field: reader.get::<$ty>()?),* })?),)*
                    _ => Err($crate::wire::Malformed),
                }
            }
        }
    };
}

pub(crate) use {wire_enum, wire_struct};

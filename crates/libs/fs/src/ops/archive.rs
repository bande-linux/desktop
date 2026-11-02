use std::cell::Cell;
use std::fmt;
use std::fs::{self, File, Metadata};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bzip2::read::MultiBzDecoder;
use bzip2::write::BzEncoder;
use flate2::read::MultiGzDecoder;
use flate2::write::GzEncoder;
use lzma_rust2::{XzOptions, XzReader, XzWriter};
use ruzstd::decoding::StreamingDecoder;
use sevenz_rust2::{ArchiveEntry, ArchiveReader, Password};
use tar::EntryType;
use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use super::{Change, Failure, Halt, Worker, claim, free_space, rename_new};
use crate::L10N;
use crate::naming::keep_both_name;
use crate::trash::remove_all;

mod sink;

use sink::{Budget, Sink, entry_path, finish_file, kept};

const BUFFER: usize = 256 * 1024;
const ZIP_LIMIT: u64 = 0xFFFF_FFFF;
const LINK_LIMIT: u64 = 4096;
const XZ_PRESET: u32 = 6;
const SEVEN_ZIP_UNIX: u32 = 0x8000;
const MODE_TYPE: u32 = 0o170_000;
const MODE_LINK: u32 = 0o120_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Zip,
    Tar,
    TarGzip,
    TarBzip2,
    TarXz,
    TarZstd,
    Gzip,
    Bzip2,
    Xz,
    Zstd,
    SevenZip,
}

const SUFFIXES: [(&str, Format); 16] = [
    (".zip", Format::Zip),
    (".tar", Format::Tar),
    (".tar.gz", Format::TarGzip),
    (".tgz", Format::TarGzip),
    (".tar.bz2", Format::TarBzip2),
    (".tbz2", Format::TarBzip2),
    (".tbz", Format::TarBzip2),
    (".tar.xz", Format::TarXz),
    (".txz", Format::TarXz),
    (".tar.zst", Format::TarZstd),
    (".tzst", Format::TarZstd),
    (".gz", Format::Gzip),
    (".bz2", Format::Bzip2),
    (".xz", Format::Xz),
    (".zst", Format::Zstd),
    (".7z", Format::SevenZip),
];

impl Format {
    pub const CREATED: [Self; 4] = [Self::Zip, Self::TarGzip, Self::TarBzip2, Self::TarXz];

    pub fn of(path: &Path) -> Option<Self> {
        split(path).map(|(format, _)| format)
    }

    pub fn suffix(self) -> &'static str {
        SUFFIXES
            .iter()
            .find(|(_, format)| *format == self)
            .map_or("", |(suffix, _)| suffix)
    }

    fn single(self) -> bool {
        matches!(self, Self::Gzip | Self::Bzip2 | Self::Xz | Self::Zstd)
    }
}

fn split(path: &Path) -> Option<(Format, String)> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let lower = name.to_lowercase();
    SUFFIXES
        .iter()
        .filter(|(suffix, _)| lower.len() > suffix.len() && lower.ends_with(suffix))
        .max_by_key(|(suffix, _)| suffix.len())
        .map(|(suffix, format)| (*format, name[..name.len() - suffix.len()].to_owned()))
}

pub fn can_extract(path: &Path) -> bool {
    Format::of(path).is_some()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    NeedsPassword,
    WrongPassword,
    Outside,
    Bomb,
    TooLarge,
}

impl Refusal {
    pub fn of(error: &io::Error) -> Option<Self> {
        error
            .get_ref()
            .and_then(|error| error.downcast_ref::<Self>())
            .copied()
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let id = match self {
            Self::NeedsPassword => "refusal-needs-password",
            Self::WrongPassword => "refusal-wrong-password",
            Self::Outside => "refusal-outside",
            Self::Bomb => "refusal-bomb",
            Self::TooLarge => "refusal-too-large",
        };
        f.write_str(&L10N.tr(id))
    }
}

impl std::error::Error for Refusal {}

impl From<Refusal> for io::Error {
    fn from(refusal: Refusal) -> Self {
        io::Error::new(io::ErrorKind::InvalidData, refusal)
    }
}

impl Failure {
    pub fn refusal(&self) -> Option<Refusal> {
        Refusal::of(&self.error)
    }
}

fn free_path(folder: &Path, name: &str, is_dir: bool) -> PathBuf {
    let path = folder.join(name);
    match path.symlink_metadata() {
        Ok(_) => keep_both_name(folder, name, is_dir),
        Err(_) => path,
    }
}

pub fn archive_path(sources: &[PathBuf], folder: &Path, format: Format) -> Option<PathBuf> {
    let name = match sources {
        [only] => only.file_name()?.to_string_lossy().into_owned(),
        _ => L10N.tr("archive-name"),
    };
    Some(free_path(
        folder,
        &format!("{name}{}", format.suffix()),
        false,
    ))
}

pub(super) enum Stop {
    Halt(Halt),
    Io(io::Error),
}

impl From<Halt> for Stop {
    fn from(halt: Halt) -> Self {
        Self::Halt(halt)
    }
}

impl From<io::Error> for Stop {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<Refusal> for Stop {
    fn from(refusal: Refusal) -> Self {
        Self::Io(refusal.into())
    }
}

impl From<ZipError> for Stop {
    fn from(error: ZipError) -> Self {
        match error {
            ZipError::InvalidPassword => Refusal::WrongPassword.into(),
            ZipError::UnsupportedArchive(ZipError::PASSWORD_REQUIRED) => {
                Refusal::NeedsPassword.into()
            }
            ZipError::Io(error) => Self::Io(error),
            error => Self::Io(io::Error::new(io::ErrorKind::InvalidData, error)),
        }
    }
}

impl From<sevenz_rust2::Error> for Stop {
    fn from(error: sevenz_rust2::Error) -> Self {
        match error {
            sevenz_rust2::Error::PasswordRequired => Refusal::NeedsPassword.into(),
            sevenz_rust2::Error::MaybeBadPassword(_) => Refusal::WrongPassword.into(),
            sevenz_rust2::Error::Io(error, _) | sevenz_rust2::Error::FileOpen(error, _) => {
                Self::Io(error)
            }
            error => Self::Io(io::Error::new(
                io::ErrorKind::InvalidData,
                error.to_string(),
            )),
        }
    }
}

struct Counted<R> {
    inner: R,
    read: Rc<Cell<u64>>,
}

impl<R: Read> Read for Counted<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.read.set(self.read.get() + read as u64);
        Ok(read)
    }
}

struct Watched<'a, R> {
    inner: R,
    worker: &'a mut Worker,
}

impl<R: Read> Read for Watched<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.worker.check().is_err() {
            return Err(io::Error::other("cancelled"));
        }
        let read = self.inner.read(buffer)?;
        self.worker.progress.bytes_done += read as u64;
        self.worker.report_progress(false);
        Ok(read)
    }
}

fn decoder<'a>(format: Format, reader: impl Read + 'a) -> io::Result<Box<dyn Read + 'a>> {
    Ok(match format {
        Format::TarGzip | Format::Gzip => Box::new(MultiGzDecoder::new(reader)),
        Format::TarBzip2 | Format::Bzip2 => Box::new(MultiBzDecoder::new(reader)),
        Format::TarXz | Format::Xz => Box::new(XzReader::new(reader, true)),
        Format::TarZstd | Format::Zstd => Box::new(
            StreamingDecoder::new(reader)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
        ),
        Format::Zip | Format::Tar | Format::SevenZip => Box::new(reader),
    })
}

fn zip_time(time: SystemTime) -> DateTime {
    jiff::Timestamp::try_from(time)
        .ok()
        .map(|stamp| stamp.to_zoned(bande_datetime::local_zone()))
        .and_then(|local| {
            DateTime::from_date_and_time(
                u16::try_from(local.year()).ok()?,
                u8::try_from(local.month()).ok()?,
                u8::try_from(local.day()).ok()?,
                u8::try_from(local.hour()).ok()?,
                u8::try_from(local.minute()).ok()?,
                u8::try_from(local.second()).ok()?,
            )
            .ok()
        })
        .unwrap_or_default()
}

fn system_time(time: DateTime) -> Option<SystemTime> {
    let civil = jiff::civil::date(
        i16::try_from(time.year()).ok()?,
        i8::try_from(time.month()).ok()?,
        i8::try_from(time.day()).ok()?,
    )
    .at(
        i8::try_from(time.hour()).ok()?,
        i8::try_from(time.minute()).ok()?,
        i8::try_from(time.second()).ok()?,
        0,
    );
    let zoned = civil.to_zoned(bande_datetime::local_zone()).ok()?;
    Some(SystemTime::from(zoned.timestamp()))
}

fn link_target(reader: &mut (impl Read + ?Sized)) -> io::Result<PathBuf> {
    let mut target = String::new();
    (&mut *reader)
        .take(LINK_LIMIT)
        .read_to_string(&mut target)?;
    Ok(PathBuf::from(target))
}

fn settle(staging: &Path, destination: &Path, stem: &str) -> io::Result<PathBuf> {
    let entries = fs::read_dir(staging)?.collect::<Result<Vec<_>, _>>()?;
    if let [only] = entries.as_slice() {
        let name = only.file_name().to_string_lossy().into_owned();
        let is_dir = only.file_type()?.is_dir();
        let (target, ()) = claim(
            || free_path(destination, &name, is_dir),
            |target| rename_new(&only.path(), target),
        )?;
        fs::remove_dir(staging)?;
        return Ok(target);
    }
    let (target, ()) = claim(
        || free_path(destination, stem, true),
        |target| rename_new(staging, target),
    )?;
    Ok(target)
}

impl Worker {
    fn pump(&mut self, from: &mut (impl Read + ?Sized), to: &mut impl Write) -> Result<(), Stop> {
        let mut buffer = vec![0; BUFFER];
        loop {
            self.check()?;
            let read = from.read(&mut buffer)?;
            if read == 0 {
                return Ok(());
            }
            to.write_all(&buffer[..read])?;
            self.progress.bytes_done += read as u64;
            self.report_progress(false);
        }
    }

    fn pump_counted(
        &mut self,
        from: &mut impl Read,
        to: &mut impl Write,
        read: &Rc<Cell<u64>>,
    ) -> Result<(), Stop> {
        let start = self.progress.bytes_done.saturating_sub(read.get());
        let mut buffer = vec![0; BUFFER];
        loop {
            self.check()?;
            let got = from.read(&mut buffer)?;
            if got == 0 {
                return Ok(());
            }
            to.write_all(&buffer[..got])?;
            self.progress.bytes_done = start + read.get();
            self.report_progress(false);
        }
    }

    pub(super) fn compress(&mut self, sources: &[PathBuf], archive: &Path) -> Result<(), Halt> {
        self.count(sources);
        let file = match File::create_new(archive) {
            Ok(file) => file,
            Err(error) => {
                self.fail(archive, error);
                return Ok(());
            }
        };
        let written = match Format::of(archive) {
            Some(Format::Zip) | None => self.write_zip(file, sources),
            Some(format) => self.write_tar(file, sources, format),
        };
        match written {
            Ok(()) => {
                self.outcome
                    .changes
                    .push(Change::Created(archive.to_path_buf()));
                Ok(())
            }
            Err(stop) => {
                let _ = fs::remove_file(archive);
                match stop {
                    Stop::Halt(halt) => Err(halt),
                    Stop::Io(error) => {
                        self.fail(archive, error);
                        Ok(())
                    }
                }
            }
        }
    }

    fn each_source(
        &mut self,
        sources: &[PathBuf],
        mut add: impl FnMut(&mut Self, &Path, String, &Metadata) -> Result<(), Stop>,
    ) -> Result<(), Stop> {
        for source in sources {
            let base = source.parent().unwrap_or(Path::new("/"));
            let mut pending = vec![source.clone()];
            while let Some(path) = pending.pop() {
                self.check()?;
                let name = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .into_owned();
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.is_dir() {
                    let mut children = fs::read_dir(&path)?
                        .map(|entry| entry.map(|entry| entry.path()))
                        .collect::<Result<Vec<_>, _>>()?;
                    children.sort();
                    pending.extend(children.into_iter().rev());
                }
                add(self, &path, name, &metadata)?;
                self.step(&path);
            }
        }
        Ok(())
    }

    fn write_zip(&mut self, file: File, sources: &[PathBuf]) -> Result<(), Stop> {
        let mut zip = ZipWriter::new(BufWriter::new(file));
        self.each_source(sources, |worker, path, name, metadata| {
            let options = SimpleFileOptions::default()
                .compression_method(CompressionMethod::Deflated)
                .unix_permissions(metadata.mode() & 0o7777)
                .last_modified_time(
                    metadata
                        .modified()
                        .map_or_else(|_| DateTime::default(), zip_time),
                );
            if metadata.is_symlink() {
                let target = fs::read_link(path)?;
                zip.add_symlink(name, target.to_string_lossy(), options)?;
            } else if metadata.is_dir() {
                zip.add_directory(format!("{name}/"), options)?;
            } else {
                zip.start_file(name, options.large_file(metadata.len() >= ZIP_LIMIT))?;
                worker.pump(&mut File::open(path)?, &mut zip)?;
            }
            Ok(())
        })?;
        zip.finish()?.flush()?;
        Ok(())
    }

    fn write_tar(&mut self, file: File, sources: &[PathBuf], format: Format) -> Result<(), Stop> {
        let out = BufWriter::new(file);
        match format {
            Format::TarGzip => self
                .tar_into(GzEncoder::new(out, flate2::Compression::default()), sources)?
                .finish()?
                .flush()?,
            Format::TarBzip2 => self
                .tar_into(BzEncoder::new(out, bzip2::Compression::default()), sources)?
                .finish()?
                .flush()?,
            Format::TarXz => self
                .tar_into(
                    XzWriter::new(out, XzOptions::with_preset(XZ_PRESET))?,
                    sources,
                )?
                .finish()?
                .flush()?,
            _ => self.tar_into(out, sources)?.flush()?,
        }
        Ok(())
    }

    fn tar_into<W: Write>(&mut self, out: W, sources: &[PathBuf]) -> Result<W, Stop> {
        let mut tar = tar::Builder::new(out);
        tar.follow_symlinks(false);
        self.each_source(sources, |worker, path, name, metadata| {
            let added = match metadata.is_file() {
                true => {
                    let mut header = tar::Header::new_gnu();
                    header.set_metadata(metadata);
                    let inner = File::open(path)?;
                    tar.append_data(&mut header, &name, Watched { inner, worker })
                }
                false => tar.append_path_with_name(path, &name),
            };
            if let Err(error) = added {
                worker.check()?;
                return Err(error.into());
            }
            Ok(())
        })?;
        Ok(tar.into_inner()?)
    }

    pub(super) fn extract(
        &mut self,
        archive: &Path,
        destination: &Path,
        password: Option<&str>,
    ) -> Result<(), Halt> {
        let Some((format, stem)) = split(archive) else {
            self.fail(
                archive,
                io::Error::new(io::ErrorKind::InvalidInput, "the file is not an archive"),
            );
            return Ok(());
        };
        let staging = match claim(
            || free_path(destination, &format!(".{stem}.extracting"), true),
            |staging| fs::create_dir(staging),
        ) {
            Ok((staging, ())) => staging,
            Err(error) => {
                self.fail(archive, error);
                return Ok(());
            }
        };
        let unpacked = self.unpack(format, archive, &staging, &stem, password);
        let settled = match unpacked {
            Ok(()) => settle(&staging, destination, &stem),
            Err(Stop::Io(error)) => Err(error),
            Err(Stop::Halt(halt)) => {
                let _ = remove_all(&staging);
                return Err(halt);
            }
        };
        match settled {
            Ok(path) => self.outcome.changes.push(Change::Created(path)),
            Err(error) => {
                let _ = remove_all(&staging);
                self.fail(archive, error);
            }
        }
        Ok(())
    }

    fn unpack(
        &mut self,
        format: Format,
        archive: &Path,
        into: &Path,
        stem: &str,
        password: Option<&str>,
    ) -> Result<(), Stop> {
        let file = File::open(archive)?;
        let metadata = file.metadata()?;
        let mut sink = Sink::open(into, Budget::new(metadata.len(), free_space(into)))?;
        match format {
            Format::Zip => self.unzip(file, &mut sink, password)?,
            Format::SevenZip => self.un_seven_zip(file, &mut sink, password)?,
            single if single.single() => {
                self.unpack_single(file, single, &mut sink, stem, metadata.mode())?;
            }
            tar => self.untar(file, tar, &mut sink)?,
        }
        sink.finish()?;
        Ok(())
    }

    fn counted(&mut self, file: File) -> io::Result<Counted<BufReader<File>>> {
        self.progress.bytes_total += file.metadata()?.len();
        Ok(Counted {
            inner: BufReader::new(file),
            read: Rc::default(),
        })
    }

    fn unpack_single(
        &mut self,
        file: File,
        format: Format,
        sink: &mut Sink,
        stem: &str,
        mode: u32,
    ) -> Result<(), Stop> {
        let counted = self.counted(file)?;
        let read = counted.read.clone();
        let mut reader = decoder(format, counted)?;
        let path = entry_path(Path::new(stem))?.ok_or(Refusal::Outside)?;
        let mut out = sink.file(&path)?;
        self.pump_counted(&mut reader, &mut sink.metered(&mut out), &read)?;
        out.set_permissions(kept(mode))?;
        self.step(&path);
        Ok(())
    }

    fn untar(&mut self, file: File, format: Format, sink: &mut Sink) -> Result<(), Stop> {
        let counted = self.counted(file)?;
        let read = counted.read.clone();
        let mut tar = tar::Archive::new(decoder(format, counted)?);
        for entry in tar.entries()? {
            self.check()?;
            let mut entry = entry?;
            let Some(path) = entry_path(&entry.path()?)? else {
                continue;
            };
            let header = entry.header();
            let mode = header.mode().ok();
            let modified = header
                .mtime()
                .ok()
                .map(|seconds| UNIX_EPOCH + Duration::from_secs(seconds));
            match header.entry_type() {
                EntryType::Directory => sink.folder(&path, mode)?,
                EntryType::Symlink => {
                    let target = entry.link_name()?.ok_or(Refusal::Outside)?;
                    sink.symlink(&path, &target)?;
                }
                EntryType::Link => {
                    let target = entry.link_name()?.ok_or(Refusal::Outside)?;
                    let target = entry_path(&target)?.ok_or(Refusal::Outside)?;
                    sink.hard_link(&path, &target)?;
                }
                EntryType::Regular | EntryType::Continuous | EntryType::GNUSparse => {
                    let mut out = sink.file(&path)?;
                    self.pump_counted(&mut entry, &mut sink.metered(&mut out), &read)?;
                    finish_file(&out, mode, modified)?;
                }
                _ => continue,
            }
            self.step(&path);
        }
        Ok(())
    }

    fn unzip(&mut self, file: File, sink: &mut Sink, password: Option<&str>) -> Result<(), Stop> {
        let mut zip = ZipArchive::new(BufReader::new(file))?;
        let declared = zip
            .decompressed_size()
            .map_or(u64::MAX, |size| u64::try_from(size).unwrap_or(u64::MAX));
        sink.budget().admits(declared)?;
        self.progress.bytes_total += declared;
        self.progress.items_total += zip.len() as u64;
        let encrypted = (0..zip.len())
            .any(|index| zip.by_index_raw(index).is_ok_and(|entry| entry.encrypted()));
        if encrypted && password.is_none() {
            return Err(Refusal::NeedsPassword.into());
        }
        for index in 0..zip.len() {
            self.check()?;
            let mut entry = match password {
                Some(password) => zip.by_index_decrypt(index, password.as_bytes())?,
                None => zip.by_index(index)?,
            };
            let Some(path) = entry_path(Path::new(&*entry.name()?))? else {
                continue;
            };
            if entry.is_dir() {
                sink.folder(&path, entry.unix_mode())?;
            } else if entry.is_symlink() {
                let target = link_target(&mut entry)?;
                sink.symlink(&path, &target)?;
            } else {
                let mut out = sink.file(&path)?;
                match self.pump(&mut entry, &mut sink.metered(&mut out)) {
                    Err(Stop::Io(error)) if entry.encrypted() && Refusal::of(&error).is_none() => {
                        return Err(Refusal::WrongPassword.into());
                    }
                    poured => poured?,
                }
                finish_file(
                    &out,
                    entry.unix_mode(),
                    entry.last_modified().and_then(system_time),
                )?;
            }
            self.step(&path);
        }
        Ok(())
    }

    fn un_seven_zip(
        &mut self,
        file: File,
        sink: &mut Sink,
        password: Option<&str>,
    ) -> Result<(), Stop> {
        let password = password.map_or_else(Password::empty, Password::new);
        let mut reader = ArchiveReader::new(BufReader::new(file), password)?;
        let files = &reader.archive().files;
        let declared = files.iter().map(|entry| entry.size).sum();
        sink.budget().admits(declared)?;
        self.progress.bytes_total += declared;
        self.progress.items_total += files.len() as u64;
        let mut stopped = None;
        reader.for_each_entries(
            |entry, data| match self.seven_zip_entry(entry, data, sink) {
                Ok(()) => Ok(true),
                Err(stop) => {
                    stopped = Some(stop);
                    Ok(false)
                }
            },
        )?;
        stopped.map_or(Ok(()), Err)
    }

    fn seven_zip_entry(
        &mut self,
        entry: &ArchiveEntry,
        data: &mut dyn Read,
        sink: &mut Sink,
    ) -> Result<(), Stop> {
        self.check()?;
        if entry.is_anti_item {
            return Ok(());
        }
        let Some(path) = entry_path(Path::new(&entry.name))? else {
            return Ok(());
        };
        let mode = (entry.windows_attributes & SEVEN_ZIP_UNIX != 0)
            .then_some(entry.windows_attributes >> 16);
        if entry.is_directory {
            sink.folder(&path, mode)?;
        } else if mode.is_some_and(|mode| mode & MODE_TYPE == MODE_LINK) {
            sink.symlink(&path, &link_target(data)?)?;
        } else {
            let mut out = sink.file(&path)?;
            self.pump(data, &mut sink.metered(&mut out))?;
            let modified = entry
                .has_last_modified_date
                .then(|| SystemTime::from(entry.last_modified_date));
            finish_file(&out, mode, modified)?;
        }
        self.step(&path);
        Ok(())
    }
}

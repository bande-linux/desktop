use std::cell::Cell;
use std::fs::{self, File};
use std::io::{self, BufReader, BufWriter, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::rc::Rc;
use std::thread;
use std::time::{Duration, SystemTime};

use bande_xdg::find_executable;
use bzip2::read::MultiBzDecoder;
use flate2::read::MultiGzDecoder;
use lzma_rust2::XzReader;
use ruzstd::decoding::StreamingDecoder;
use zip::result::ZipError;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, DateTime, ZipArchive, ZipWriter};

use super::{Change, Halt, Worker};
use crate::L10N;
use crate::naming::keep_both_name;
use crate::trash::remove_all;

const BUFFER: usize = 256 * 1024;
const TOOL_POLL: Duration = Duration::from_millis(50);
const ZIP_LIMIT: u64 = 0xFFFF_FFFF;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Packing {
    Zip,
    Tar,
    Gzip,
    Xz,
    Bzip2,
    Zstd,
    SevenZip,
}

const SUFFIXES: [(&str, Packing); 12] = [
    (".zip", Packing::Zip),
    (".tar", Packing::Tar),
    (".tar.gz", Packing::Gzip),
    (".tgz", Packing::Gzip),
    (".tar.xz", Packing::Xz),
    (".txz", Packing::Xz),
    (".tar.bz2", Packing::Bzip2),
    (".tbz2", Packing::Bzip2),
    (".tbz", Packing::Bzip2),
    (".tar.zst", Packing::Zstd),
    (".tzst", Packing::Zstd),
    (".7z", Packing::SevenZip),
];

fn packing(path: &Path) -> Option<(Packing, String)> {
    let name = path.file_name()?.to_string_lossy().into_owned();
    let lower = name.to_lowercase();
    SUFFIXES
        .iter()
        .filter(|(suffix, _)| lower.len() > suffix.len() && lower.ends_with(suffix))
        .max_by_key(|(suffix, _)| suffix.len())
        .map(|(suffix, packing)| (*packing, name[..name.len() - suffix.len()].to_owned()))
}

fn seven_zip_tool(archive: &Path, into: &Path) -> Option<Vec<String>> {
    let archive = archive.to_string_lossy().into_owned();
    let into = into.to_string_lossy().into_owned();
    if let Some(program) = ["7zz", "7z", "7za"]
        .iter()
        .find_map(|name| find_executable(name))
    {
        return Some(vec![
            program.to_string_lossy().into_owned(),
            "x".to_owned(),
            "-y".to_owned(),
            format!("-o{into}"),
            archive,
        ]);
    }
    find_executable("bsdtar").map(|program| {
        vec![
            program.to_string_lossy().into_owned(),
            "-xf".to_owned(),
            archive,
            "-C".to_owned(),
            into,
        ]
    })
}

pub fn can_extract(path: &Path) -> bool {
    match packing(path) {
        Some((Packing::SevenZip, _)) => seven_zip_tool(path, path).is_some(),
        Some(_) => true,
        None => false,
    }
}

fn free_path(folder: &Path, name: &str, is_dir: bool) -> PathBuf {
    let path = folder.join(name);
    match path.symlink_metadata() {
        Ok(_) => keep_both_name(folder, name, is_dir),
        Err(_) => path,
    }
}

pub fn archive_path(sources: &[PathBuf]) -> Option<PathBuf> {
    let folder = sources.first()?.parent()?;
    let name = match sources {
        [only] => only.file_name()?.to_string_lossy().into_owned(),
        _ => L10N.tr("archive-name"),
    };
    Some(free_path(folder, &format!("{name}.zip"), false))
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

impl From<ZipError> for Stop {
    fn from(error: ZipError) -> Self {
        Self::Io(io::Error::other(error))
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

fn passes_through_link(into: &Path, relative: &Path) -> bool {
    let mut path = into.to_path_buf();
    relative.components().any(|part| {
        path.push(part);
        path.symlink_metadata()
            .is_ok_and(|metadata| metadata.file_type().is_symlink())
    })
}

fn settle(staging: &Path, destination: &Path, stem: &str) -> io::Result<PathBuf> {
    let entries = fs::read_dir(staging)?.collect::<Result<Vec<_>, _>>()?;
    if let [only] = entries.as_slice() {
        let name = only.file_name().to_string_lossy().into_owned();
        let target = free_path(destination, &name, only.file_type()?.is_dir());
        fs::rename(only.path(), &target)?;
        fs::remove_dir(staging)?;
        return Ok(target);
    }
    let target = free_path(destination, stem, true);
    fs::rename(staging, &target)?;
    Ok(target)
}

impl Worker {
    fn pump(&mut self, from: &mut impl Read, to: &mut impl Write) -> Result<(), Stop> {
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

    pub(super) fn compress(&mut self, sources: &[PathBuf], archive: &Path) -> Result<(), Halt> {
        self.count(sources);
        let file = match File::create_new(archive) {
            Ok(file) => file,
            Err(error) => {
                self.fail(archive, error);
                return Ok(());
            }
        };
        match self.write_zip(file, sources) {
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

    fn write_zip(&mut self, file: File, sources: &[PathBuf]) -> Result<(), Stop> {
        let mut zip = ZipWriter::new(BufWriter::new(file));
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
                let options = SimpleFileOptions::default()
                    .compression_method(CompressionMethod::Deflated)
                    .unix_permissions(metadata.mode() & 0o7777)
                    .last_modified_time(
                        metadata
                            .modified()
                            .map_or_else(|_| DateTime::default(), zip_time),
                    );
                if metadata.is_symlink() {
                    let target = fs::read_link(&path)?;
                    zip.add_symlink(name, target.to_string_lossy(), options)?;
                } else if metadata.is_dir() {
                    zip.add_directory(format!("{name}/"), options)?;
                    let mut children = fs::read_dir(&path)?
                        .map(|entry| entry.map(|entry| entry.path()))
                        .collect::<Result<Vec<_>, _>>()?;
                    children.sort();
                    pending.extend(children.into_iter().rev());
                } else {
                    zip.start_file(name, options.large_file(metadata.len() >= ZIP_LIMIT))?;
                    self.pump(&mut File::open(&path)?, &mut zip)?;
                }
                self.step(&path);
            }
        }
        zip.finish()?.flush()?;
        Ok(())
    }

    pub(super) fn extract(&mut self, archive: &Path, destination: &Path) -> Result<(), Halt> {
        let Some((packing, stem)) = packing(archive) else {
            self.fail(
                archive,
                io::Error::new(io::ErrorKind::InvalidInput, "the file is not an archive"),
            );
            return Ok(());
        };
        let staging = free_path(destination, &format!(".{stem}.extracting"), true);
        let unpacked = fs::create_dir(&staging)
            .map_err(Stop::from)
            .and_then(|()| self.unpack(packing, archive, &staging));
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

    fn unpack(&mut self, packing: Packing, archive: &Path, into: &Path) -> Result<(), Stop> {
        if packing == Packing::Zip {
            return self.unzip(archive, into);
        }
        if packing == Packing::SevenZip {
            return self.run_tool(archive, into);
        }
        let file = File::open(archive)?;
        self.progress.bytes_total += file.metadata()?.len();
        let read = Rc::new(Cell::new(0));
        let counted = Counted {
            inner: BufReader::new(file),
            read: read.clone(),
        };
        let reader: Box<dyn Read> = match packing {
            Packing::Gzip => Box::new(MultiGzDecoder::new(counted)),
            Packing::Xz => Box::new(XzReader::new(counted, true)),
            Packing::Bzip2 => Box::new(MultiBzDecoder::new(counted)),
            Packing::Zstd => Box::new(
                StreamingDecoder::new(counted)
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?,
            ),
            _ => Box::new(counted),
        };
        let mut tar = tar::Archive::new(reader);
        tar.set_preserve_permissions(true);
        tar.set_preserve_mtime(true);
        tar.set_unpack_xattrs(false);
        let start = self.progress.bytes_done;
        for entry in tar.entries()? {
            self.check()?;
            let mut entry = entry?;
            entry.unpack_in(into)?;
            self.progress.bytes_done = start + read.get();
            let path = into.join(entry.path()?);
            self.step(&path);
        }
        Ok(())
    }

    fn unzip(&mut self, archive: &Path, into: &Path) -> Result<(), Stop> {
        let mut zip = ZipArchive::new(BufReader::new(File::open(archive)?))?;
        for index in 0..zip.len() {
            let entry = zip.by_index(index)?;
            self.progress.bytes_total += entry.size();
            self.progress.items_total += 1;
        }
        for index in 0..zip.len() {
            self.check()?;
            let mut entry = zip.by_index(index)?;
            let Some(relative) = entry.enclosed_name() else {
                continue;
            };
            if passes_through_link(into, &relative) {
                return Err(Stop::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the archive writes through a link",
                )));
            }
            let target = into.join(relative);
            if entry.is_dir() {
                fs::create_dir_all(&target)?;
                continue;
            }
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent)?;
            }
            if entry.is_symlink() {
                let mut link = String::new();
                entry.read_to_string(&mut link)?;
                symlink(link, &target)?;
            } else {
                let mut out = File::create_new(&target)?;
                self.pump(&mut entry, &mut out)?;
                if let Some(mode) = entry.unix_mode() {
                    out.set_permissions(fs::Permissions::from_mode(mode & 0o777))?;
                }
                if let Some(modified) = entry.last_modified().and_then(system_time) {
                    out.set_modified(modified)?;
                }
            }
            self.step(&target);
        }
        Ok(())
    }

    fn run_tool(&mut self, archive: &Path, into: &Path) -> Result<(), Stop> {
        let Some(command) = seven_zip_tool(archive, into) else {
            return Err(Stop::Io(io::Error::new(
                io::ErrorKind::Unsupported,
                "no program to open 7-Zip archives is installed",
            )));
        };
        let mut child = Command::new(&command[0])
            .args(&command[1..])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        loop {
            if let Some(status) = child.try_wait()? {
                return match status.success() {
                    true => Ok(()),
                    false => Err(Stop::Io(io::Error::other(format!(
                        "{} could not open the archive",
                        command[0]
                    )))),
                };
            }
            if let Err(halt) = self.check() {
                let _ = child.kill();
                let _ = child.wait();
                return Err(Stop::Halt(halt));
            }
            thread::sleep(TOOL_POLL);
        }
    }
}

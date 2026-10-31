reps = {
 'crates/libs/fs/src/trash.rs': [
  ('''fn not_found(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, message)
}''', '''fn not_found(message: &str) -> io::Error {
    crate::user_error(io::ErrorKind::NotFound, message)
}'''),
  ('not_found("the item has no name")', 'not_found("error-no-name")'),
  ('not_found("the item has no folder")', 'not_found("error-no-folder")'),
  ('''            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "the volume's trash folder is not usable",
            ));''', '''            return Err(crate::user_error(
                io::ErrorKind::PermissionDenied,
                "error-trash-unusable",
            ));'''),
  ('''        Err(io::Error::other("no free name in the trash"))''', '''        Err(crate::user_error(io::ErrorKind::Other, "error-trash-no-free-name"))'''),
  ('''    io::Error::new(
        io::ErrorKind::CrossesDevices,
        "refusing to delete across a mount point",
    )''', '''    crate::user_error(io::ErrorKind::CrossesDevices, "error-across-mount")'''),
  ('''            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "the item is already in the trash",
            ));''', '''            return Err(crate::user_error(
                io::ErrorKind::InvalidInput,
                "error-already-in-trash",
            ));'''),
  ('''        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "an item with that name already exists",
        ));''', '''        return Err(crate::user_error(
            io::ErrorKind::AlreadyExists,
            "error-name-taken",
        ));'''),
 ],
 'crates/libs/fs/src/ops.rs': [
  ('''io::Error::new(io::ErrorKind::InvalidInput, "the item has no name")''', '''crate::user_error(io::ErrorKind::InvalidInput, "error-no-name")'''),
  ('''                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "an item can't replace the folder that contains it",
                        ),''', '''                        crate::user_error(
                            io::ErrorKind::InvalidInput,
                            "error-replace-own-folder",
                        ),'''),
  ('''                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a folder can't be copied into itself",
                ),''', '''                crate::user_error(io::ErrorKind::InvalidInput, "error-copy-into-itself"),'''),
  ('''                io::Error::new(io::ErrorKind::InvalidInput, "the item has no folder"),''', '''                crate::user_error(io::ErrorKind::InvalidInput, "error-no-folder"),'''),
  ('''            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "special files can't be copied",
            ))''', '''            Err(crate::user_error(
                io::ErrorKind::Unsupported,
                "error-special-file",
            ))'''),
  ('''                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "a folder can't be moved into itself",
                ),''', '''                crate::user_error(io::ErrorKind::InvalidInput, "error-move-into-itself"),'''),
  ('''                        io::Error::new(
                            io::ErrorKind::AlreadyExists,
                            "an item with that name already exists",
                        ),''', '''                        crate::user_error(io::ErrorKind::AlreadyExists, "error-name-taken"),'''),
  ('''        self.outcome.failures.push(Failure {
            path: path.to_path_buf(),
            error,
        });''', '''        self.outcome.failures.push(Failure {
            path: path.to_path_buf(),
            error: crate::described(error),
        });'''),
 ],
 'crates/libs/fs/src/ops/archive.rs': [
  ('''                io::Error::new(io::ErrorKind::InvalidInput, "the file is not an archive"),''', '''                crate::user_error(io::ErrorKind::InvalidInput, "error-not-an-archive"),'''),
  ('''                return Err(Stop::Io(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "the archive writes through a link",
                )));''', '''                return Err(Stop::Io(crate::user_error(
                    io::ErrorKind::InvalidData,
                    "error-archive-through-link",
                )));'''),
  ('''            return Err(Stop::Io(io::Error::new(
                io::ErrorKind::Unsupported,
                "no program to open 7-Zip archives is installed",
            )));''', '''            return Err(Stop::Io(crate::user_error(
                io::ErrorKind::Unsupported,
                "error-no-7-zip",
            )));'''),
 ],
}
for p, rs in reps.items():
    s = open(p).read()
    for a, b in rs:
        assert a in s, (p, a)
        s = s.replace(a, b)
    open(p, 'w').write(s)

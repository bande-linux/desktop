use std::process::ExitCode;

fn main() -> ExitCode {
    bande_log::init("bande-backup-restore");
    bande_backup::recovery::main(std::env::args().skip(1).collect())
}

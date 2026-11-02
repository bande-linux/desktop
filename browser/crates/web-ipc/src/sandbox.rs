use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};

use landlock::{
    ABI, Access, AccessFs, AccessNet, CompatLevel, Compatible, Ruleset, RulesetAttr,
    RulesetCreatedAttr, RulesetStatus, Scope, path_beneath_rules,
};
use rustix::process::DumpableBehavior;
use rustix::thread::UnshareFlags;
use seccompiler::{
    BpfProgram, SeccompAction, SeccompCmpArgLen, SeccompCmpOp, SeccompCondition, SeccompFilter,
    SeccompRule, TargetArch,
};

const LANDLOCK: ABI = ABI::V9;

pub fn unshare_namespaces() -> bool {
    let flags = UnshareFlags::NEWUSER | UnshareFlags::NEWNET | UnshareFlags::NEWIPC;
    #[allow(unsafe_code)]
    // SAFETY: the flags do not include `FILES` or `FS`, the only ones whose effect on sibling
    // threads makes `unshare` unsafe; new user, network and IPC namespaces change no state that
    // Rust code relies on.
    unsafe { rustix::thread::unshare_unsafe(flags) }.is_ok()
}

pub fn font_directories() -> Vec<PathBuf> {
    let mut directories: Vec<PathBuf> = [
        "/usr/share/fonts",
        "/usr/local/share/fonts",
        "/etc/fonts",
        "/var/cache/fontconfig",
    ]
    .into_iter()
    .map(PathBuf::from)
    .collect();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        directories.push(home.join(".fonts"));
        directories.push(home.join(".local/share/fonts"));
        directories.push(home.join(".cache/fontconfig"));
    }
    for (variable, suffix) in [("XDG_DATA_HOME", "fonts"), ("XDG_CACHE_HOME", "fontconfig")] {
        if let Some(base) = std::env::var_os(variable).map(PathBuf::from) {
            directories.push(base.join(suffix));
        }
    }
    directories
}

pub fn enter(readable: &[PathBuf]) -> io::Result<()> {
    rustix::process::set_dumpable_behavior(DumpableBehavior::NotDumpable)?;
    restrict_files(readable)?;
    restrict_syscalls()
}

fn denied(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, error.to_string())
}

fn restrict_files(readable: &[PathBuf]) -> io::Result<()> {
    let existing = readable.iter().filter(|path| Path::exists(path));
    let status = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(AccessFs::from_all(LANDLOCK))
        .and_then(|ruleset| ruleset.handle_access(AccessNet::from_all(LANDLOCK)))
        .and_then(|ruleset| ruleset.scope(Scope::from_all(LANDLOCK)))
        .and_then(|ruleset| ruleset.create())
        .and_then(|ruleset| {
            ruleset.add_rules(path_beneath_rules(
                existing,
                AccessFs::ReadFile | AccessFs::ReadDir,
            ))
        })
        .and_then(|ruleset| ruleset.restrict_self())
        .map_err(denied)?;
    match status.ruleset {
        RulesetStatus::NotEnforced => Err(denied("Landlock is not available")),
        _ if !status.no_new_privs => Err(denied("no_new_privs is not set")),
        _ => Ok(()),
    }
}

#[cfg(target_arch = "x86_64")]
const ARCH_SYSCALLS: &[libc::c_long] = &[
    libc::SYS_open,
    libc::SYS_stat,
    libc::SYS_lstat,
    libc::SYS_access,
    libc::SYS_readlink,
    libc::SYS_poll,
    libc::SYS_getdents,
];

#[cfg(not(target_arch = "x86_64"))]
const ARCH_SYSCALLS: &[libc::c_long] = &[];

const SYSCALLS: &[libc::c_long] = &[
    libc::SYS_read,
    libc::SYS_readv,
    libc::SYS_write,
    libc::SYS_writev,
    libc::SYS_pread64,
    libc::SYS_pwrite64,
    libc::SYS_lseek,
    libc::SYS_close,
    libc::SYS_openat,
    libc::SYS_fstat,
    libc::SYS_newfstatat,
    libc::SYS_statx,
    libc::SYS_fstatfs,
    libc::SYS_statfs,
    libc::SYS_getdents64,
    libc::SYS_readlinkat,
    libc::SYS_faccessat,
    libc::SYS_faccessat2,
    libc::SYS_fcntl,
    libc::SYS_ftruncate,
    libc::SYS_memfd_create,
    libc::SYS_mmap,
    libc::SYS_munmap,
    libc::SYS_mprotect,
    libc::SYS_mremap,
    libc::SYS_madvise,
    libc::SYS_brk,
    libc::SYS_futex,
    libc::SYS_sendmsg,
    libc::SYS_recvmsg,
    libc::SYS_ppoll,
    libc::SYS_clock_gettime,
    libc::SYS_clock_getres,
    libc::SYS_gettimeofday,
    libc::SYS_getrandom,
    libc::SYS_rt_sigreturn,
    libc::SYS_rt_sigprocmask,
    libc::SYS_rt_sigaction,
    libc::SYS_sigaltstack,
    libc::SYS_restart_syscall,
    libc::SYS_exit,
    libc::SYS_exit_group,
    libc::SYS_getpid,
    libc::SYS_gettid,
    libc::SYS_getuid,
    libc::SYS_geteuid,
    libc::SYS_getgid,
    libc::SYS_getegid,
    libc::SYS_uname,
    libc::SYS_sched_yield,
    libc::SYS_sched_getaffinity,
];

fn restrict_syscalls() -> io::Result<()> {
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = SYSCALLS
        .iter()
        .chain(ARCH_SYSCALLS)
        .map(|&syscall| (syscall, Vec::new()))
        .collect();
    let own = u64::from(
        rustix::process::getpid()
            .as_raw_nonzero()
            .get()
            .unsigned_abs(),
    );
    let to_self = SeccompCondition::new(0, SeccompCmpArgLen::Dword, SeccompCmpOp::Eq, own)
        .and_then(|condition| SeccompRule::new(vec![condition]))
        .map_err(denied)?;
    rules.insert(libc::SYS_tgkill, vec![to_self]);
    let arch = TargetArch::try_from(std::env::consts::ARCH).map_err(denied)?;
    let filter = SeccompFilter::new(
        rules,
        SeccompAction::Errno(libc::EPERM as u32),
        SeccompAction::Allow,
        arch,
    )
    .map_err(denied)?;
    let program = BpfProgram::try_from(filter).map_err(denied)?;
    seccompiler::apply_filter_all_threads(&program).map_err(denied)
}

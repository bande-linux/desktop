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

const NETWORK_READABLE: &[&str] = &[
    "/etc/resolv.conf",
    "/etc/hosts",
    "/etc/host.conf",
    "/etc/nsswitch.conf",
    "/etc/gai.conf",
    "/etc/services",
    "/etc/ld.so.cache",
    "/etc/ssl",
    "/etc/ca-certificates",
    "/etc/pki",
    "/usr/share/ca-certificates",
    "/usr/local/share/ca-certificates",
    "/usr/lib",
    "/lib",
    "/usr/share/publicsuffix",
    "/run/systemd/resolve",
];

const NETWORK_SOCKETS: &[&str] = &["/run/systemd/resolve", "/run/nscd", "/run/avahi-daemon"];

pub fn enter(readable: &[PathBuf]) -> io::Result<()> {
    rustix::process::set_dumpable_behavior(DumpableBehavior::NotDumpable)?;
    restrict_files(&Files {
        readable,
        writable: None,
        sockets: &[],
        network: false,
    })?;
    restrict_syscalls(Profile::Renderer)
}

pub fn enter_network(storage: Option<&Path>) -> io::Result<()> {
    rustix::process::set_dumpable_behavior(DumpableBehavior::NotDumpable)?;
    let readable: Vec<PathBuf> = NETWORK_READABLE.iter().map(PathBuf::from).collect();
    let sockets: Vec<PathBuf> = NETWORK_SOCKETS.iter().map(PathBuf::from).collect();
    restrict_files(&Files {
        readable: &readable,
        writable: storage,
        sockets: &sockets,
        network: true,
    })?;
    restrict_syscalls(Profile::Network)
}

fn denied(error: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, error.to_string())
}

struct Files<'a> {
    readable: &'a [PathBuf],
    writable: Option<&'a Path>,
    sockets: &'a [PathBuf],
    network: bool,
}

fn restrict_files(files: &Files<'_>) -> io::Result<()> {
    let existing = |paths: &'_ [PathBuf]| -> Vec<PathBuf> {
        paths.iter().filter(|path| path.exists()).cloned().collect()
    };
    let read = AccessFs::ReadFile | AccessFs::ReadDir;
    let ruleset = Ruleset::default()
        .set_compatibility(CompatLevel::BestEffort)
        .handle_access(AccessFs::from_all(LANDLOCK))
        .and_then(|ruleset| match files.network {
            true => Ok(ruleset),
            false => ruleset.handle_access(AccessNet::from_all(LANDLOCK)),
        })
        .and_then(|ruleset| ruleset.scope(Scope::from_all(LANDLOCK)))
        .and_then(|ruleset| ruleset.create())
        .and_then(|ruleset| ruleset.add_rules(path_beneath_rules(existing(files.readable), read)))
        .and_then(|ruleset| {
            ruleset.add_rules(path_beneath_rules(
                existing(files.sockets),
                AccessFs::ResolveUnix,
            ))
        })
        .and_then(|ruleset| {
            let writable: Vec<PathBuf> =
                files.writable.map(Path::to_path_buf).into_iter().collect();
            let access = AccessFs::from_all(LANDLOCK)
                & !AccessFs::Execute
                & !AccessFs::MakeChar
                & !AccessFs::MakeBlock
                & !AccessFs::IoctlDev;
            ruleset.add_rules(path_beneath_rules(existing(&writable), access))
        });
    let status = ruleset
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum Profile {
    Renderer,
    Network,
}

#[cfg(target_arch = "x86_64")]
const NETWORK_ARCH_SYSCALLS: &[libc::c_long] = &[libc::SYS_epoll_wait];

#[cfg(not(target_arch = "x86_64"))]
const NETWORK_ARCH_SYSCALLS: &[libc::c_long] = &[];

const NETWORK_SYSCALLS: &[libc::c_long] = &[
    libc::SYS_connect,
    libc::SYS_bind,
    libc::SYS_getsockname,
    libc::SYS_getpeername,
    libc::SYS_setsockopt,
    libc::SYS_getsockopt,
    libc::SYS_sendto,
    libc::SYS_recvfrom,
    libc::SYS_sendmmsg,
    libc::SYS_recvmmsg,
    libc::SYS_shutdown,
    libc::SYS_epoll_create1,
    libc::SYS_epoll_ctl,
    libc::SYS_epoll_pwait,
    libc::SYS_epoll_pwait2,
    libc::SYS_eventfd2,
    libc::SYS_timerfd_create,
    libc::SYS_timerfd_settime,
    libc::SYS_timerfd_gettime,
    libc::SYS_clone3,
    libc::SYS_set_robust_list,
    libc::SYS_rseq,
    libc::SYS_nanosleep,
    libc::SYS_clock_nanosleep,
    libc::SYS_fsync,
    libc::SYS_fdatasync,
    libc::SYS_renameat,
    libc::SYS_renameat2,
    libc::SYS_unlinkat,
    libc::SYS_mkdirat,
    libc::SYS_flock,
];

fn rule(conditions: &[(u8, SeccompCmpOp, u64)]) -> io::Result<SeccompRule> {
    let conditions = conditions
        .iter()
        .map(|(argument, operator, value)| {
            SeccompCondition::new(*argument, SeccompCmpArgLen::Dword, operator.clone(), *value)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(denied)?;
    SeccompRule::new(conditions).map_err(denied)
}

fn install(
    rules: BTreeMap<i64, Vec<SeccompRule>>,
    mismatch: SeccompAction,
    matched: SeccompAction,
) -> io::Result<()> {
    let arch = TargetArch::try_from(std::env::consts::ARCH).map_err(denied)?;
    let filter = SeccompFilter::new(rules, mismatch, matched, arch).map_err(denied)?;
    let program = BpfProgram::try_from(filter).map_err(denied)?;
    seccompiler::apply_filter_all_threads(&program).map_err(denied)
}

fn restrict_syscalls(profile: Profile) -> io::Result<()> {
    let extra: &[libc::c_long] = match profile {
        Profile::Renderer => &[],
        Profile::Network => NETWORK_SYSCALLS,
    };
    let arch_extra: &[libc::c_long] = match profile {
        Profile::Renderer => &[],
        Profile::Network => NETWORK_ARCH_SYSCALLS,
    };
    let mut rules: BTreeMap<i64, Vec<SeccompRule>> = SYSCALLS
        .iter()
        .chain(ARCH_SYSCALLS)
        .chain(extra)
        .chain(arch_extra)
        .map(|&syscall| (syscall, Vec::new()))
        .collect();
    let own = u64::from(
        rustix::process::getpid()
            .as_raw_nonzero()
            .get()
            .unsigned_abs(),
    );
    rules.insert(libc::SYS_tgkill, vec![rule(&[(0, SeccompCmpOp::Eq, own)])?]);
    if profile == Profile::Network {
        let thread = libc::CLONE_THREAD as u64;
        rules.insert(
            libc::SYS_clone,
            vec![rule(&[(0, SeccompCmpOp::MaskedEq(thread), thread)])?],
        );
        let domains = [libc::AF_UNIX, libc::AF_INET, libc::AF_INET6];
        let mut sockets = domains
            .iter()
            .map(|domain| rule(&[(0, SeccompCmpOp::Eq, *domain as u64)]))
            .collect::<io::Result<Vec<_>>>()?;
        sockets.push(rule(&[
            (0, SeccompCmpOp::Eq, libc::AF_NETLINK as u64),
            (2, SeccompCmpOp::Eq, libc::NETLINK_ROUTE as u64),
        ])?);
        rules.insert(libc::SYS_socket, sockets);
        let requests = [libc::FIONBIO, libc::FIONREAD];
        let ioctls = requests
            .iter()
            .map(|request| rule(&[(1, SeccompCmpOp::Eq, *request)]))
            .collect::<io::Result<Vec<_>>>()?;
        rules.insert(libc::SYS_ioctl, ioctls);
        rules.insert(
            libc::SYS_prctl,
            vec![rule(&[(0, SeccompCmpOp::Eq, libc::PR_SET_NAME as u64)])?],
        );
        let no_clone3 = BTreeMap::from([(libc::SYS_clone3, Vec::new())]);
        install(
            no_clone3,
            SeccompAction::Allow,
            SeccompAction::Errno(libc::ENOSYS as u32),
        )?;
    }
    install(
        rules,
        SeccompAction::Errno(libc::EPERM as u32),
        SeccompAction::Allow,
    )
}

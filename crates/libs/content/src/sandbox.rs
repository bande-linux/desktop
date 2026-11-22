const LOAD: u16 = 0x20;
const JUMP_EQUAL: u16 = 0x15;
const RETURN: u16 = 0x06;
const ARCH_OFFSET: u32 = 4;
const NUMBER_OFFSET: u32 = 0;
const ALLOW: u32 = 0x7fff_0000;
const KILL: u32 = 0x8000_0000;
const DENY: u32 = 0x0005_0000 | libc::EPERM as u32;

#[cfg(target_arch = "aarch64")]
const ARCH: u32 = 0xc000_00b7;
#[cfg(target_arch = "x86_64")]
const ARCH: u32 = 0xc000_003e;

const ALLOWED: &[libc::c_long] = &[
    libc::SYS_read,
    libc::SYS_write,
    libc::SYS_close,
    libc::SYS_exit,
    libc::SYS_exit_group,
    libc::SYS_mmap,
    libc::SYS_munmap,
    libc::SYS_mremap,
    libc::SYS_mprotect,
    libc::SYS_madvise,
    libc::SYS_brk,
    libc::SYS_futex,
    libc::SYS_rt_sigreturn,
    libc::SYS_rt_sigprocmask,
    libc::SYS_sigaltstack,
    libc::SYS_getrandom,
    libc::SYS_clock_gettime,
];

const THREADS: &[libc::c_long] = &[
    libc::SYS_clone3,
    libc::SYS_set_robust_list,
    libc::SYS_rseq,
    libc::SYS_sched_getaffinity,
];

fn statement(code: u16, k: u32) -> libc::sock_filter {
    libc::sock_filter {
        code,
        jt: 0,
        jf: 0,
        k,
    }
}

fn program(allowed: &[libc::c_long]) -> Vec<libc::sock_filter> {
    let count = allowed.len();
    let mut filter = vec![
        statement(LOAD, ARCH_OFFSET),
        libc::sock_filter {
            code: JUMP_EQUAL,
            jt: 1,
            jf: 0,
            k: ARCH,
        },
        statement(RETURN, KILL),
        statement(LOAD, NUMBER_OFFSET),
    ];
    for (index, number) in allowed.iter().enumerate() {
        filter.push(libc::sock_filter {
            code: JUMP_EQUAL,
            jt: (count - index) as u8,
            jf: 0,
            k: *number as u32,
        });
    }
    filter.push(statement(RETURN, DENY));
    filter.push(statement(RETURN, ALLOW));
    filter
}

/// Allows this process only to compute, allocate, read and write open descriptors and exit from
/// now on; any other system call fails. Returns whether the filter is in place.
pub fn only_compute_from_now_on() -> bool {
    confine_to(ALLOWED)
}

pub(crate) fn only_compute_and_start_threads_from_now_on() -> bool {
    confine_to(&[ALLOWED, THREADS].concat())
}

fn confine_to(allowed: &[libc::c_long]) -> bool {
    let filter = program(allowed);
    let Ok(len) = u16::try_from(filter.len()) else {
        return false;
    };
    let program = libc::sock_fprog {
        len,
        filter: filter.as_ptr().cast_mut(),
    };
    // SAFETY: program points to filter, a valid BPF program of len instructions that outlives both
    // prctl calls; the kernel copies it.
    unsafe {
        libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) == 0
            && libc::prctl(
                libc::PR_SET_SECCOMP,
                libc::SECCOMP_MODE_FILTER,
                &raw const program,
            ) == 0
    }
}

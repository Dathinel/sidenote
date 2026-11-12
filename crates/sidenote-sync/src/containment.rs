//! Linux limits are installed after fork and before exec, with no spawn/attach gap.
use std::{io, os::unix::process::CommandExt, process::Command};

pub const MEMORY_LIMIT: u64 = 512 * 1024 * 1024;
pub const CPU_SECONDS: u64 = 120;

pub fn configure(command: &mut Command) {
    let parent = unsafe { libc::getpid() };
    // Only async-signal-safe syscalls may run in this closure.
    unsafe {
        command.pre_exec(move || {
            if libc::setpgid(0, 0) != 0
                || libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0
                || libc::getppid() != parent
                || libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0
            {
                return Err(io::Error::other("WORKER_CONTAINMENT_FAILED"));
            }
            for (resource, value) in [
                (libc::RLIMIT_AS, MEMORY_LIMIT),
                (libc::RLIMIT_CPU, CPU_SECONDS),
                (libc::RLIMIT_CORE, 0),
            ] {
                let limit = libc::rlimit {
                    rlim_cur: value,
                    rlim_max: value,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
}

pub fn verify() -> io::Result<()> {
    let mut limit = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    let mut death = 0i32;
    unsafe {
        if libc::getrlimit(libc::RLIMIT_AS, &mut limit) != 0
            || limit.rlim_max > MEMORY_LIMIT
            || libc::getrlimit(libc::RLIMIT_CPU, &mut limit) != 0
            || limit.rlim_max > CPU_SECONDS
            || libc::prctl(libc::PR_GET_PDEATHSIG, &mut death) != 0
            || death != libc::SIGKILL
            || libc::getpgrp() != libc::getpid()
        {
            return Err(io::Error::other("WORKER_CONTAINMENT_REQUIRED"));
        }
    }
    Ok(())
}

pub fn kill_group(pid: u32) {
    unsafe {
        libc::kill(-(pid as i32), libc::SIGKILL);
    }
}

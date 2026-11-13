//! Descendants share the caller's sandbox and cgroup; PID-namespace teardown reaps escapes.
use std::{
    ffi::OsString,
    fs, io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    process::Command,
    time::{Duration, Instant},
};
fn error() -> io::Error {
    io::Error::other("PID_NAMESPACE_CONTAINMENT_UNAVAILABLE")
}
fn dead(fd: &OwnedFd) -> bool {
    let mut poll = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    unsafe { libc::poll(&mut poll, 1, 0) != 0 }
}
pub(crate) fn run(args: &[OsString]) -> io::Result<i32> {
    if args.is_empty() {
        return Err(error());
    }
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    if uid == 0 {
        return Err(error());
    }
    let parent = unsafe { libc::getppid() };
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, libc::getpid(), 0) as i32 };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let front = unsafe { OwnedFd::from_raw_fd(fd) };
    // A private user namespace permits private PID and empty network namespaces.
    // Keep the numeric identity: exec as a nonzero uid drops its namespace capabilities.
    if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWPID | libc::CLONE_NEWNET) } != 0
    {
        return Err(io::Error::last_os_error());
    }
    fs::write("/proc/self/setgroups", "deny")?;
    fs::write("/proc/self/uid_map", format!("{uid} {uid} 1"))?;
    fs::write("/proc/self/gid_map", format!("{gid} {gid} 1"))?;
    if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) } != 0
        || unsafe { libc::getppid() } != parent
    {
        return Err(error());
    }
    // This executable is single-threaded; no fork occurs in the multithreaded server.
    let init = unsafe { libc::fork() };
    if init < 0 {
        return Err(io::Error::last_os_error());
    }
    if init == 0 {
        let result = (|| -> io::Result<i32> {
            if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) } != 0 || dead(&front) {
                return Err(error());
            }
            let mut command = Command::new(&args[0]);
            command.args(&args[1..]);
            crate::containment::configure(&mut command);
            let status = command.status()?;
            Ok(status.code().unwrap_or(1))
        })();
        // Exiting namespace PID 1 makes the kernel kill *all* remaining descendants,
        // including grandchildren which called setsid or entered another PID namespace.
        unsafe {
            libc::_exit(result.unwrap_or(2));
        }
    }
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let mut status = 0;
        let waited = unsafe { libc::waitpid(init, &mut status, libc::WNOHANG) };
        if waited == init {
            return Ok(if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                2
            });
        }
        if waited < 0 {
            return Err(io::Error::last_os_error());
        }
        if Instant::now() >= deadline {
            unsafe {
                libc::kill(init, libc::SIGKILL);
                libc::waitpid(init, &mut status, 0);
            }
            return Ok(2);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

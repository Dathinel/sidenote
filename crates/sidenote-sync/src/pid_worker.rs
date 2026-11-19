//! Descendants share the caller's sandbox and cgroup; PID-namespace teardown reaps escapes.
use std::{
    ffi::OsString,
    fs, io,
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    process::Command,
    time::{Duration, Instant},
};
mod root;
pub use root::AgentUnavailable;
enum Profile {
    Offline,
    AgentNotes(fs::File),
    Speech,
}
/// Dormant trusted setup seam; call only in a single-threaded disposable process.
/// `root` pins a private staged tree containing only runtime/meeting and empty mountpoints.
pub fn agent_notes(args: &[OsString], root: fs::File) -> Result<i32, AgentUnavailable> {
    run_profile(args, Profile::AgentNotes(root)).map_err(|e| root::reason(&e))
}
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
    run_profile(args, Profile::Offline)
}
pub(crate) fn speech(args: &[OsString]) -> io::Result<i32> {
    run_profile(args, Profile::Speech)
}
fn run_profile(args: &[OsString], profile: Profile) -> io::Result<i32> {
    let agent = matches!(profile, Profile::AgentNotes(_));
    let speech = matches!(profile, Profile::Speech);
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
    root::namespaces(agent)?;
    fs::write("/proc/self/setgroups", "deny")?;
    fs::write("/proc/self/uid_map", format!("{uid} {uid} 1"))?;
    fs::write("/proc/self/gid_map", format!("{gid} {gid} 1"))?;
    if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) } != 0
        || unsafe { libc::getppid() } != parent
    {
        return Err(error());
    }
    // This executable is single-threaded; no fork occurs in the multithreaded server.
    let mut pipe = [-1; 2];
    if unsafe { libc::pipe2(pipe.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(error());
    }
    let (rx, tx) = unsafe { (OwnedFd::from_raw_fd(pipe[0]), OwnedFd::from_raw_fd(pipe[1])) };
    let init = unsafe { libc::fork() };
    if init < 0 {
        return Err(io::Error::last_os_error());
    }
    if init == 0 {
        let result = (|| -> io::Result<i32> {
            if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) } != 0 || dead(&front) {
                return Err(io::Error::other(
                    AgentUnavailable::ChildDeathCheckUnavailable,
                ));
            }
            drop(front);
            drop(rx);
            if let Profile::AgentNotes(root) = profile {
                root::enter(root).map_err(io::Error::other)?;
            }
            let mut command = Command::new(&args[0]);
            command.args(&args[1..]);
            if agent {
                command.env_clear().current_dir("/work");
            }
            if speech {
                crate::containment::configure_speech(&mut command);
            } else {
                crate::containment::configure(&mut command);
            }
            let status = command
                .status()
                .map_err(|_| io::Error::other(AgentUnavailable::ExecSetupUnavailable))?;
            Ok(status.code().unwrap_or(1))
        })();
        // Exiting namespace PID 1 makes the kernel kill *all* remaining descendants,
        // including grandchildren which called setsid or entered another PID namespace.
        unsafe {
            let code = result.as_ref().err().map_or(0, |e| root::reason(e) as u8);
            libc::write(tx.as_raw_fd(), [code].as_ptr().cast(), 1);
            libc::_exit(result.unwrap_or(2));
        }
    }
    drop(tx);
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let mut status = 0;
        let waited = unsafe { libc::waitpid(init, &mut status, libc::WNOHANG) };
        if waited == init {
            let mut reason = [0u8];
            if unsafe { libc::read(rx.as_raw_fd(), reason.as_mut_ptr().cast(), 1) } != 1 {
                return Err(io::Error::other(
                    AgentUnavailable::ProcessIsolationUnavailable,
                ));
            }
            if reason[0] != 0 {
                return Err(io::Error::other(root::decode(reason[0])));
            }
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

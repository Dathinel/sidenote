//! Explicit desktop-user execution; never used by the headless worker adapter.
use std::{
    io,
    process::{Child, Command, Stdio},
};

/// Owns the helper tree. Drop cancels the tree and reaps the immediate child.
pub struct UserRunner {
    child: Child,
    pub stdin: Option<std::process::ChildStdin>,
    pub stdout: Option<std::process::ChildStdout>,
    pub stderr: Option<std::process::ChildStderr>,
    #[cfg(windows)]
    job: Option<super::ProcessJob>,
}
impl UserRunner {
    /// Inherits user context; stdin is piped or null, stdout/stderr are piped.
    /// Linux hosts must call `supervisor_entry` before application initialization.
    pub fn spawn(command: &mut Command, piped_stdin: bool) -> io::Result<Self> {
        #[cfg(target_os = "linux")]
        let mut command = {
            let mut host = Command::new(std::env::current_exe()?);
            host.arg("--desktop-notes-supervisor")
                .arg(command.get_program())
                .args(command.get_args());
            if let Some(cwd) = command.get_current_dir() {
                host.current_dir(cwd);
            }
            for (key, value) in command.get_envs() {
                if let Some(value) = value {
                    host.env(key, value);
                } else {
                    host.env_remove(key);
                }
            }
            linux::configure(&mut host, libc::SIGTERM, false);
            host
        };
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        #[cfg(not(any(windows, target_os = "linux")))]
        return Err(super::unavailable());
        let mut child = command
            .stdin(if piped_stdin {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        #[cfg(windows)]
        let job = match super::ProcessJob::attach(&child) {
            Ok(job) => Some(job),
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            stderr: child.stderr.take(),
            child,
            #[cfg(windows)]
            job,
        })
    }
    pub fn id(&self) -> u32 {
        self.child.id()
    }
    pub fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.child.try_wait()
    }
    pub fn wait(&mut self) -> io::Result<std::process::ExitStatus> {
        drop(self.stdin.take());
        self.child.wait()
    }
    pub fn stop(&mut self) -> io::Result<()> {
        #[cfg(windows)]
        drop(self.job.take());
        if self.child.try_wait()?.is_none() {
            #[cfg(target_os = "linux")]
            if unsafe { libc::kill(self.child.id() as i32, libc::SIGTERM) } != 0 {
                return Err(io::Error::last_os_error());
            }
            #[cfg(windows)]
            let _ = self.child.kill();
            self.child.wait()?;
        }
        Ok(())
    }
}
impl Drop for UserRunner {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

#[cfg(target_os = "linux")]
pub fn supervisor_entry() {
    let mut args = std::env::args_os().skip(1);
    if args.next().as_deref() == Some(std::ffi::OsStr::new("--desktop-notes-supervisor")) {
        // Runs before settings, listeners, UI or any application threads exist.
        std::process::exit(linux::supervise(args).unwrap_or(125));
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::os::unix::process::CommandExt;
    pub(super) fn configure(command: &mut Command, signal: i32, helper: bool) {
        let parent = unsafe { libc::getpid() };
        unsafe {
            command.pre_exec(move || {
                let mut mask = std::mem::zeroed();
                libc::sigemptyset(&mut mask);
                libc::sigaddset(&mut mask, libc::SIGTERM);
                // Block before installing PDEATHSIG so death during exec is queued.
                if libc::sigprocmask(
                    if helper {
                        libc::SIG_UNBLOCK
                    } else {
                        libc::SIG_BLOCK
                    },
                    &mask,
                    std::ptr::null_mut(),
                ) != 0
                    || libc::setpgid(0, 0) != 0
                    || libc::prctl(libc::PR_SET_PDEATHSIG, signal) != 0
                    || libc::getppid() != parent
                {
                    return Err(io::Error::from_raw_os_error(libc::EPERM));
                }
                Ok(())
            });
        }
    }
    pub(super) fn supervise(mut args: impl Iterator<Item = std::ffi::OsString>) -> io::Result<i32> {
        unsafe {
            let mut death = 0;
            if libc::prctl(libc::PR_GET_PDEATHSIG, &mut death) != 0
                || death != libc::SIGTERM
                || libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1) != 0
            {
                return Err(super::super::unavailable());
            }
        }
        let program = args.next().ok_or(io::ErrorKind::InvalidInput)?;
        let mut command = Command::new(program);
        command.args(args);
        configure(&mut command, libc::SIGKILL, true);
        let mut helper = command.spawn()?;
        let pid = helper.id() as i32;
        let result = unsafe {
            let mut mask = std::mem::zeroed();
            libc::sigemptyset(&mut mask);
            libc::sigaddset(&mut mask, libc::SIGTERM);
            loop {
                let mut info: libc::siginfo_t = std::mem::zeroed();
                // Keep the exited group leader pinned until after killpg (no PID reuse).
                if libc::waitid(
                    libc::P_PID,
                    pid as u32,
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                ) != 0
                {
                    break Err(io::Error::last_os_error());
                }
                if info.si_pid() != 0 {
                    break Ok(());
                }
                let timeout = libc::timespec {
                    tv_sec: 0,
                    tv_nsec: 10_000_000,
                };
                if libc::sigtimedwait(&mask, std::ptr::null_mut(), &timeout) == libc::SIGTERM {
                    break Ok(());
                }
            }
        };
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
        let status = helper.wait()?;
        // As subreaper, collect orphaned descendants that stayed in the helper group.
        loop {
            let waited = unsafe { libc::waitpid(-pid, std::ptr::null_mut(), 0) };
            if waited < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            if waited <= 0 {
                break;
            }
        }
        result?;
        Ok(status.code().unwrap_or(128))
    }
}

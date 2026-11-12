//! A transient service per worker, nested slices for aggregate descendant budgets.
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read},
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const UNAVAILABLE: &str = "UNAVAILABLE_ON_THIS_LINUX_SETUP: helpers require systemd user services with delegated memory, pids and cpu controllers";
const WALL_SECONDS: u64 = 120;

fn unavailable() -> io::Error {
    io::Error::new(io::ErrorKind::Unsupported, UNAVAILABLE)
}

pub fn owner_unit() -> io::Result<(String, PathBuf)> {
    let text = fs::read_to_string("/proc/self/cgroup")?;
    let group = text
        .lines()
        .find_map(|line| line.strip_prefix("0::"))
        .ok_or_else(unavailable)?;
    let path = PathBuf::from(group);
    let manager = format!("user@{}.service", unsafe { libc::geteuid() });
    if !path
        .components()
        .any(|part| part.as_os_str() == OsStr::new(&manager))
    {
        return Err(unavailable());
    }
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .ok_or_else(unavailable)?;
    if !name.ends_with(".service")
        || name.starts_with("user@")
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.@".contains(&b))
    {
        return Err(unavailable());
    }
    Ok((name.to_owned(), path))
}

pub fn command(program: &OsStr) -> io::Result<Command> {
    owner_unit().map_err(|_| unavailable())?;
    let executable = std::env::current_exe()?;
    let parent = executable.parent().ok_or_else(unavailable)?;
    let folder = if parent
        .file_name()
        .is_some_and(|n| n == "deps" || n == "examples")
    {
        parent.parent().ok_or_else(unavailable)?
    } else {
        parent
    };
    let helper = folder.join("sidenote-linux-worker");
    if !helper.is_file() {
        return Err(unavailable());
    }
    let mut command = Command::new(helper);
    command.arg("--launch").arg(program);
    Ok(command)
}

/// Small infrastructure probes use a deadline without entering a worker slice.
pub fn bounded_output(command: &mut Command, limit: Duration) -> io::Result<std::process::Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    crate::containment::configure(command);
    let mut child = command.spawn()?;
    let mut stdout = child.stdout.take().ok_or_else(unavailable)?;
    if unsafe { libc::fcntl(stdout.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0 {
        crate::containment::kill_group(child.id());
        let _ = child.wait();
        return Err(unavailable());
    }
    let deadline = Instant::now() + limit;
    let mut bytes = Vec::new();
    loop {
        let mut buffer = [0; 4096];
        match stdout.read(&mut buffer) {
            Ok(n) => bytes.extend_from_slice(&buffer[..n]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
            Err(_) => {
                crate::containment::kill_group(child.id());
                let _ = child.wait();
                return Err(unavailable());
            }
        }
        let status = match child.try_wait() {
            Ok(status) => status,
            Err(_) => {
                crate::containment::kill_group(child.id());
                let _ = child.wait();
                return Err(unavailable());
            }
        };
        if let Some(status) = status {
            crate::containment::kill_group(child.id());
            return Ok(std::process::Output {
                status,
                stdout: bytes,
                stderr: Vec::new(),
            });
        }
        if Instant::now() >= deadline || bytes.len() > 8192 {
            crate::containment::kill_group(child.id());
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "LINUX_PROBE_DEADLINE",
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn control(args: &[&OsStr]) -> io::Result<Vec<u8>> {
    let output = bounded_output(
        Command::new("/usr/bin/systemctl").arg("--user").args(args),
        Duration::from_secs(2),
    )?;
    if !output.status.success() {
        return Err(unavailable());
    }
    Ok(output.stdout)
}

fn start_time(pid: u32) -> io::Result<String> {
    fs::read_to_string(format!("/proc/{pid}/stat"))?
        .rsplit(')')
        .next()
        .and_then(|rest| rest.split_whitespace().nth(19))
        .map(str::to_owned)
        .ok_or_else(unavailable)
}

fn pidfd(pid: u32, expected: &OsStr) -> io::Result<OwnedFd> {
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) as i32 };
    if fd < 0 {
        return Err(unavailable());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    if OsStr::new(&start_time(pid)?) != expected {
        return Err(unavailable());
    }
    Ok(fd)
}

fn slice_limits(slice: &str) -> io::Result<()> {
    control(&[OsStr::new("start"), OsStr::new(slice)])?;
    control(&[
        OsStr::new("set-property"),
        OsStr::new("--runtime"),
        OsStr::new(slice),
        OsStr::new("MemoryMax=512M"),
        OsStr::new("TasksMax=32"),
        OsStr::new("CPUQuota=100%"),
    ])?;
    Ok(())
}

fn launch(args: &[OsString]) -> io::Result<i32> {
    let (owner, path) = owner_unit()?;
    let parent_slice = path
        .parent()
        .and_then(Path::file_name)
        .and_then(OsStr::to_str)
        .filter(|name| {
            name.starts_with("sidenote-w")
                && name.ends_with(".slice")
                && name
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b))
        });
    let nonce = &hex::encode(crate::identity::random32())[..16];
    let stem = parent_slice
        .map(|s| s.trim_end_matches(".slice"))
        .unwrap_or("sidenote");
    let slice = format!("{stem}-w{nonce}.slice");
    let unit = format!("sidenote-worker-{nonce}.service");
    if let Err(error) = slice_limits(&slice) {
        let _ = control(&[OsStr::new("stop"), OsStr::new(&slice)]);
        return Err(error);
    }
    let front = std::process::id();
    let parent = unsafe { libc::getppid() } as u32;
    let mut run = Command::new("/usr/bin/systemd-run");
    run.args([
        "--user",
        "--quiet",
        "--pipe",
        "--wait",
        "--collect",
        "--service-type=exec",
    ])
    .arg(format!("--unit={unit}"))
    .arg(format!("--slice={slice}"))
    .arg(format!("--property=BindsTo={owner}"))
    .arg(format!("--property=After={owner}"))
    .args([
        "--property=MemoryMax=512M",
        "--property=TasksMax=32",
        "--property=CPUQuota=100%",
        "--property=RuntimeMaxSec=120s",
        "--property=TimeoutStopSec=1s",
        "--property=KillMode=control-group",
        "--property=KillSignal=SIGKILL",
        "--property=NoNewPrivileges=yes",
        "--property=LimitAS=536870912",
        "--property=LimitCPU=120",
        "--property=LimitCORE=0",
        "--property=UMask=0077",
        "--property=OOMPolicy=stop",
    ])
    .arg("--working-directory")
    .arg(std::env::current_dir()?);
    run.arg("--")
        .arg(std::env::current_exe()?)
        .arg("--guard")
        .arg(parent.to_string())
        .arg(start_time(parent)?)
        .arg(front.to_string())
        .arg(start_time(front)?)
        .arg(&slice)
        .args(args);
    let result = run.status();
    let _ = control(&[OsStr::new("stop"), OsStr::new(&slice)]);
    result.map(|status| status.code().unwrap_or(1))
}

fn guard(args: &[OsString]) -> io::Result<i32> {
    if args.len() < 6 {
        return Err(unavailable());
    }
    let owner = pidfd(
        args[0]
            .to_string_lossy()
            .parse()
            .map_err(|_| unavailable())?,
        &args[1],
    )?;
    let front = pidfd(
        args[2]
            .to_string_lossy()
            .parse()
            .map_err(|_| unavailable())?,
        &args[3],
    )?;
    let (_, group) = owner_unit()?;
    let parent = group.parent().ok_or_else(unavailable)?;
    if parent.file_name() != Some(args[4].as_os_str()) {
        return Err(unavailable());
    }
    let directory =
        Path::new("/sys/fs/cgroup").join(parent.strip_prefix("/").map_err(|_| unavailable())?);
    for (file, maximum) in [
        ("memory.max", crate::containment::MEMORY_LIMIT),
        ("pids.max", 32),
    ] {
        let value: u64 = fs::read_to_string(directory.join(file))?
            .trim()
            .parse()
            .map_err(|_| unavailable())?;
        if value == 0 || value > maximum {
            return Err(unavailable());
        }
    }
    let cpu = fs::read_to_string(directory.join("cpu.max"))?;
    let cpu: Vec<_> = cpu.split_whitespace().collect();
    if cpu.len() != 2
        || cpu[0] == "max"
        || cpu[0].parse::<u64>().map_err(|_| unavailable())?
            > cpu[1].parse::<u64>().map_err(|_| unavailable())?
    {
        return Err(unavailable());
    }
    let mut command = Command::new(&args[5]);
    command.args(&args[6..]);
    // Do not serialize API-key environment values into argv or transient unit files.
    // Read the live launcher's private environment and pass it only to the real child.
    use std::os::unix::ffi::OsStringExt;
    let front_pid: u32 = args[2]
        .to_string_lossy()
        .parse()
        .map_err(|_| unavailable())?;
    let environment = zeroize::Zeroizing::new(fs::read(format!("/proc/{front_pid}/environ"))?);
    command.env_clear();
    for entry in environment.split(|b| *b == 0).filter(|e| !e.is_empty()) {
        let Some(equal) = entry.iter().position(|b| *b == b'=') else {
            return Err(unavailable());
        };
        let name = OsString::from_vec(entry[..equal].to_vec());
        let mut value = OsString::from_vec(entry[equal + 1..].to_vec());
        if name == "CREDENTIALS_DIRECTORY" {
            let directory = Path::new(&value);
            if !directory.is_absolute() {
                return Err(unavailable());
            }
            value = Path::new(&format!("/proc/{front_pid}/root"))
                .join(directory.strip_prefix("/").map_err(|_| unavailable())?)
                .into_os_string();
        }
        command.env(name, value);
    }
    crate::containment::configure(&mut command);
    let mut child = command.spawn()?;
    let deadline = Instant::now() + Duration::from_secs(WALL_SECONDS);
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status.code().unwrap_or(1));
        }
        let mut fds = [
            libc::pollfd {
                fd: owner.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: front.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), 2, 25) };
        if ready != 0 || Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(2); // MainPID exit makes systemd kill every remaining descendant.
        }
    }
}

pub fn run(args: Vec<OsString>) -> io::Result<i32> {
    match args.first().and_then(|a| a.to_str()) {
        Some("--launch") if args.len() >= 2 => launch(&args[1..]),
        Some("--guard") => guard(&args[1..]),
        _ => Err(unavailable()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn infrastructure_probe_has_a_short_deadline() {
        let started = Instant::now();
        let result = bounded_output(
            Command::new("/bin/sleep").arg("30"),
            Duration::from_millis(100),
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}

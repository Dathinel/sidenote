//! Helpers inherit the sandbox and the aggregate resource budget of their owner.
use std::{
    ffi::{OsStr, OsString},
    fs,
    io::{self, Read},
    os::fd::AsRawFd,
    path::PathBuf,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const UNAVAILABLE: &str = "UNAVAILABLE_ON_THIS_LINUX_SETUP: helpers require systemd user services with delegated memory, pids and cpu controllers";

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
    command_profile(program, false)
}

fn command_profile(program: &OsStr, speech: bool) -> io::Result<Command> {
    if matches!(
        std::path::Path::new(program)
            .file_stem()
            .and_then(OsStr::to_str),
        Some("claude" | "codex" | "opencode" | "opencode2")
    ) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "agent runners need network; not yet supported on Linux",
        ));
    }
    budget(speech).map_err(|_| unavailable())?;
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
    command
        .arg(if speech { "--speech" } else { "--launch" })
        .arg(program);
    Ok(command)
}

/// Native speech uses the same namespace launcher with a separately bounded memory profile.
pub fn speech_command(program: &OsStr) -> io::Result<Command> {
    command_profile(program, true)
}

pub fn spawn_speech(command: &mut Command) -> io::Result<std::process::Child> {
    let expected = command_profile(OsStr::new("unused"), true)?;
    if command.get_program() != expected.get_program()
        || command.get_args().next() != Some(OsStr::new("--speech"))
    {
        return Err(unavailable());
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::containment::configure_speech(command);
    command.spawn()
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

fn budget(speech: bool) -> io::Result<()> {
    let (_, group) = owner_unit()?;
    let directory = std::path::Path::new("/sys/fs/cgroup")
        .join(group.strip_prefix("/").map_err(|_| unavailable())?);
    for (file, maximum) in [
        (
            "memory.max",
            if speech {
                1024 * 1024 * 1024
            } else {
                crate::containment::MEMORY_LIMIT
            },
        ),
        ("pids.max", if speech { 64 } else { 32 }),
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
    Ok(())
}

pub fn run(args: Vec<OsString>) -> io::Result<i32> {
    match args.first().and_then(|a| a.to_str()) {
        Some("--direct" | "--launch") if args.len() >= 2 => {
            budget(false)?;
            crate::pid_worker::run(&args[1..])
        }
        Some("--speech") if args.len() >= 2 => {
            budget(true)?;
            crate::pid_worker::speech(&args[1..])
        }
        _ => Err(unavailable()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packaged_service_admits_ordinary_helpers() {
        let unit = include_str!("../../../packaging/systemd/sidenote-server@.service");
        let property = |name: &str| {
            let prefix = format!("--property={name}=");
            unit.split_whitespace()
                .find_map(|word| word.strip_prefix(&prefix))
                .unwrap()
        };
        let memory = property("MemoryMax");
        let (number, suffix) = memory.split_at(memory.len() - 1);
        let bytes = number.parse::<u64>().unwrap()
            * match suffix {
                "M" => 1024 * 1024,
                "G" => 1024 * 1024 * 1024,
                _ => panic!("unsupported packaged memory unit"),
            };
        let (_, group) = owner_unit().unwrap();
        let directory =
            std::path::Path::new("/sys/fs/cgroup").join(group.strip_prefix("/").unwrap());
        assert_eq!(
            fs::read_to_string(directory.join("memory.max"))
                .unwrap()
                .trim()
                .parse::<u64>()
                .unwrap(),
            bytes,
            "run this test with the packaged worker's actual limits"
        );
        assert_eq!(
            fs::read_to_string(directory.join("pids.max"))
                .unwrap()
                .trim(),
            property("TasksMax")
        );
        assert_eq!(property("CPUQuota"), "100%%");
        speech_command(OsStr::new("/bin/true")).unwrap();
        let ordinary = command(OsStr::new("/bin/true")).unwrap();
        assert_eq!(ordinary.get_args().next(), Some(OsStr::new("--launch")));
    }
    #[test]
    fn speech_keeps_the_agent_runner_refusal() {
        for program in ["claude", "codex", "opencode", "opencode2"] {
            let error = speech_command(OsStr::new(program)).unwrap_err();
            assert!(error.to_string().contains("agent runners need network"));
        }
    }
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

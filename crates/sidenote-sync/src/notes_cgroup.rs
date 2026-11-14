use super::notes_supervisor::Failure;
use std::{fs, path::PathBuf};
const CONTROLLERS: [&str; 3] = ["memory", "pids", "cpu"];
const AGGREGATE: [(&str, &str); 4] = [
    ("memory.max", "536870912"),
    ("memory.swap.max", "0"),
    ("pids.max", "32"),
    ("cpu.max", "100000 100000"),
];
const JOB: [(&str, &str); 4] = [
    ("memory.max", "469762048"),
    ("memory.swap.max", "0"),
    ("pids.max", "28"),
    ("cpu.max", "80000 100000"),
];
fn unavailable<T>(_: T) -> Failure {
    Failure::CgroupBudgetUnavailable
}
fn read(path: impl AsRef<std::path::Path>) -> Result<String, Failure> {
    fs::read_to_string(path).map_err(unavailable)
}
fn controllers(text: &str) -> Result<(), Failure> {
    if CONTROLLERS
        .iter()
        .all(|c| text.split_whitespace().any(|s| s == *c))
    {
        Ok(())
    } else {
        Err(Failure::CgroupBudgetUnavailable)
    }
}
fn membership(text: &str) -> Result<&str, Failure> {
    let path = text
        .strip_suffix('\n')
        .unwrap_or(text)
        .strip_prefix("0::/")
        .ok_or(Failure::CgroupBudgetUnavailable)?;
    if path.chars().any(char::is_control)
        || (!path.is_empty() && path.split('/').any(|p| matches!(p, "" | "." | "..")))
    {
        return Err(Failure::CgroupBudgetUnavailable);
    }
    Ok(path)
}
fn current(pid: &str) -> Result<PathBuf, Failure> {
    let text = read(format!("/proc/{pid}/cgroup"))?;
    let path = membership(&text)?;
    let root = PathBuf::from("/sys/fs/cgroup");
    let path = fs::canonicalize(root.join(path)).map_err(unavailable)?;
    if !path.starts_with(&root) {
        return Err(Failure::CgroupBudgetUnavailable);
    }
    Ok(path)
}
fn limits(path: &std::path::Path, values: &[(&str, &str)]) -> Result<(), Failure> {
    for (name, value) in values {
        fs::write(path.join(name), value).map_err(unavailable)?;
        if read(path.join(name))?.trim() != *value {
            return Err(Failure::CgroupBudgetUnavailable);
        }
    }
    Ok(())
}
fn leaf_name(nonce: u128) -> String {
    format!("job-{nonce:032x}")
}
pub struct Budget(PathBuf);
pub struct Leaf(pub PathBuf);
impl Budget {
    pub fn prepare(unit: &str) -> Result<Self, Failure> {
        let here = current("self")?;
        let unit = here
            .ancestors()
            .find(|p| p.file_name().is_some_and(|n| n == unit))
            .ok_or(Failure::CgroupBudgetUnavailable)?;
        controllers(&read(unit.join("cgroup.controllers"))?)?;
        let root = unit.join("notes");
        let own = root.join("supervisor");
        fs::create_dir_all(&own).map_err(unavailable)?;
        fs::write(own.join("cgroup.procs"), std::process::id().to_string()).map_err(unavailable)?;
        if current("self")? != own {
            return Err(Failure::CgroupBudgetUnavailable);
        }
        for parent in [unit, root.as_path()] {
            fs::write(parent.join("cgroup.subtree_control"), "+memory +pids +cpu")
                .map_err(unavailable)?;
            controllers(&read(parent.join("cgroup.subtree_control"))?)?;
            if !read(parent.join("cgroup.procs"))?.trim().is_empty() {
                return Err(Failure::CgroupBudgetUnavailable);
            }
        }
        // Box total: unchanged server cap (512 MiB/32 tasks) + notes 512 MiB/32 tasks.
        limits(&root, &AGGREGATE)?;
        limits(&root, &[("memory.min", "16777216")])?;
        limits(
            &own,
            &[
                ("memory.max", "67108864"),
                ("memory.min", "16777216"), // 16 MiB covers the measured supervisor need; see box receipt.
                ("memory.swap.max", "0"),
                ("pids.max", "4"),
                ("cpu.max", "20000 100000"),
            ],
        )?;
        Ok(Self(root))
    }
    pub fn job(&self, nonce: u128) -> Result<Leaf, Failure> {
        if current("self")? != self.0.join("supervisor") {
            return Err(Failure::CgroupBudgetUnavailable);
        }
        controllers(&read(self.0.join("cgroup.subtree_control"))?)?;
        let leaf = Leaf(self.0.join(leaf_name(nonce)));
        fs::create_dir(&leaf.0).map_err(unavailable)?;
        if let Err(e) = limits(&leaf.0, &JOB) {
            fs::remove_dir(&leaf.0).map_err(unavailable)?;
            return Err(e);
        }
        Ok(leaf)
    }
}
impl Leaf {
    pub fn contains(&self, pid: u32) -> Result<(), Failure> {
        if current(&pid.to_string())? != self.0 {
            return Err(Failure::CgroupBudgetUnavailable);
        }
        Ok(())
    }
    pub fn kill(&self) -> Result<(), Failure> {
        fs::write(self.0.join("cgroup.kill"), "1").map_err(unavailable)
    }
    pub async fn remove(&self) -> Result<(), Failure> {
        self.kill()?;
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while !read(self.0.join("cgroup.events"))?
                .lines()
                .any(|s| s == "populated 0")
            {
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            fs::remove_dir(&self.0).map_err(unavailable)
        })
        .await
        .map_err(|_| Failure::CgroupCleanupTimeout)?
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contract() {
        assert_eq!(leaf_name(u128::MAX), "job-ffffffffffffffffffffffffffffffff");
        assert_eq!(leaf_name(1), "job-00000000000000000000000000000001");
        assert_eq!(
            JOB.map(|(_, v)| v),
            ["469762048", "0", "28", "80000 100000"]
        );
        assert_eq!(
            AGGREGATE.map(|(_, v)| v),
            ["536870912", "0", "32", "100000 100000"]
        );
        for missing in ["cpu pids", "memory pids", "cpu memory"] {
            assert_eq!(controllers(missing), Err(Failure::CgroupBudgetUnavailable));
        }
        controllers("cpu io memory pids").unwrap();
        for valid in ["0::/", "0::/a/b", "0::/a/b\n"] {
            membership(valid).unwrap();
        }
        for bad in [
            "",
            "0::relative",
            "1::/a",
            "0:cpu:/a",
            "0::/a\n0::/a\n",
            "0::/a\n1:cpu:/b",
            "0::/a\n\n",
            "0::/a\r\n",
            "0::/../a",
            "0:://a",
            "0::/a/",
        ] {
            assert_eq!(membership(bad), Err(Failure::CgroupBudgetUnavailable));
        }
    }
    #[tokio::test(start_paused = true)]
    async fn cleanup_timeout() {
        let temp = tempfile::tempdir().unwrap();
        let leaf = Leaf(temp.path().to_owned());
        fs::write(leaf.0.join("cgroup.events"), "populated 1\n").unwrap();
        assert_eq!(leaf.remove().await, Err(Failure::CgroupCleanupTimeout));
        assert_eq!(read(leaf.0.join("cgroup.kill")).unwrap(), "1");
        assert!(leaf.0.exists());
    }
    #[tokio::test(flavor = "current_thread")]
    #[ignore = "requires an owned delegated system unit"]
    async fn live() {
        use std::{
            os::{
                fd::AsRawFd,
                unix::process::{CommandExt, ExitStatusExt},
            },
            process::Command,
        };
        let unit = std::env::var("SIDENOTE_TEST_UNIT").unwrap();
        if std::env::var_os("SIDENOTE_TEST_NO_DELEGATE").is_some() {
            assert!(matches!(
                Budget::prepare(&unit),
                Err(Failure::CgroupBudgetUnavailable)
            ));
            println!("PASS CgroupBudgetUnavailable without delegation");
            return;
        }
        let budget = Budget::prepare(&unit).unwrap();
        assert_eq!(
            read(budget.0.join("supervisor/memory.min")).unwrap().trim(),
            "16777216"
        );
        assert_eq!(
            unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) },
            0
        );
        for (nonce, program) in [
            (1, "a=bytearray(480*1024*1024)"),
            (
                2,
                "import os,errno\np=[]\ntry:\n while True:\n  c=os.fork()\n  if c==0:\n   import time; time.sleep(20); os._exit(0)\n  p.append(c)\nexcept OSError as e:\n assert e.errno==errno.EAGAIN\nfinally:\n for c in p: os.kill(c,9); os.waitpid(c,0)\nassert len(p)==27",
            ),
            (
                3,
                "import os,time\nr,w=os.pipe(); c=os.fork()\nif c:\n os.close(w); assert os.read(r,1)==b'R'; os.waitpid(c,0)\nelse:\n c=os.fork()\n if c: os._exit(0)\n os.setsid(); os.write(w,b'R'); time.sleep(30); os._exit(0)",
            ),
        ] {
            let leaf = budget.job(nonce).unwrap();
            let fd = fs::OpenOptions::new()
                .write(true)
                .open(leaf.0.join("cgroup.procs"))
                .unwrap();
            let mut cmd = Command::new("/usr/bin/python3");
            cmd.arg("-c").arg(program);
            unsafe {
                cmd.pre_exec(move || {
                    if libc::write(fd.as_raw_fd(), b"0".as_ptr().cast(), 1) != 1 {
                        return Err(std::io::Error::last_os_error());
                    }
                    Ok(())
                });
            }
            let status = cmd.status().unwrap();
            if nonce == 1 {
                assert_eq!(status.signal(), Some(9));
                assert!(
                    read(leaf.0.join("memory.events"))
                        .unwrap()
                        .contains("oom_kill 1")
                );
            } else if nonce == 2 {
                assert!(status.success());
                assert!(read(leaf.0.join("pids.events")).unwrap().contains("max 1"));
            } else {
                assert!(status.success());
                assert!(
                    read(leaf.0.join("cgroup.events"))
                        .unwrap()
                        .contains("populated 1")
                );
            }
            assert_eq!(current("self").unwrap(), budget.0.join("supervisor"));
            leaf.remove().await.unwrap();
            if nonce == 3 {
                let mut status = 0;
                assert!(unsafe { libc::waitpid(-1, &mut status, 0) } > 0);
                assert_eq!(libc::WTERMSIG(status), libc::SIGKILL);
            }
            assert!(!leaf.0.exists());
            println!("PASS job {nonce}: limit enforced; supervisor alive; leaf removed after reap");
        }
        let peak = read(budget.0.join("supervisor/memory.peak")).unwrap();
        assert!(peak.trim().parse::<u64>().unwrap() < 16777216);
        let status = read("/proc/self/status").unwrap();
        println!(
            "MEASURE supervisor cgroup peak={}; {}",
            peak.trim(),
            status.lines().find(|s| s.starts_with("VmHWM:")).unwrap()
        );
    }
}

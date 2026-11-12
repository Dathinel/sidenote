use std::{
    fs, io,
    path::{Path, PathBuf},
};
pub fn show_error(_message: &str) {
    eprintln!("{}", super::UNAVAILABLE);
}
pub fn process_running(pid: u32) -> bool {
    unsafe {
        libc::kill(pid as i32, 0) == 0
            || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }
}
pub fn hex_running() -> io::Result<bool> {
    Err(super::unavailable())
}
pub fn updater_child() -> io::Result<Option<u32>> {
    Err(super::unavailable())
}
pub fn replace_file(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)?;
    fs::File::open(to.parent().ok_or(io::ErrorKind::InvalidInput)?)?.sync_all()
}
pub fn publish_file(from: &Path, to: &Path) -> io::Result<()> {
    fs::hard_link(from, to)?;
    fs::File::open(to.parent().ok_or(io::ErrorKind::InvalidInput)?)?.sync_all()
}
pub struct Instance;
pub fn single_instance() -> io::Result<Option<Instance>> {
    Err(super::unavailable())
}
// Commands are configured before spawn by the Unix CommandExt adapter.
pub struct ProcessJob(u32);
impl ProcessJob {
    pub fn attach(child: &std::process::Child) -> io::Result<Self> {
        let pid = child.id();
        #[cfg(target_os = "linux")]
        if !std::fs::read_link(format!("/proc/{pid}/exe"))
            .is_ok_and(|p| p.file_name() == Some(std::ffi::OsStr::new("sidenote-linux-worker")))
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                sidenote_sync::tree_worker::UNAVAILABLE,
            ));
        }
        if unsafe { libc::getpgid(pid as i32) } != pid as i32 {
            return Err(io::Error::other("WORKER_CONTAINMENT_REQUIRED"));
        }
        Ok(Self(pid))
    }
}
impl Drop for ProcessJob {
    fn drop(&mut self) {
        #[cfg(target_os = "linux")]
        sidenote_sync::containment::kill_group(self.0);
    }
}
pub fn set_window_icon(_hwnd: isize) {}
pub fn fit_to_work_area(_hwnd: isize, want: (f32, f32)) -> (f32, f32) {
    want
}
pub fn open_folder(_path: &Path) -> io::Result<()> {
    Err(super::unavailable())
}
pub fn reveal_file(_path: &Path) -> io::Result<()> {
    Err(super::unavailable())
}
pub fn guard_window_close(_hwnd: isize, _request: impl Fn() + 'static) -> io::Result<()> {
    Err(super::unavailable())
}
pub fn save_html_dialog(_name: &str) -> io::Result<Option<PathBuf>> {
    Err(super::unavailable())
}
pub fn copy_file(_path: &Path) -> io::Result<()> {
    Err(super::unavailable())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use crate::platform::CommandExt;
    #[test]
    fn direct_process_limits_do_not_substitute_for_a_worker_scope() -> io::Result<()> {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("60")
            .creation_flags(0)
            .spawn()?;
        let error = ProcessJob::attach(&child)
            .err()
            .expect("direct helpers must fail closed");
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        let limits = fs::read_to_string(format!("/proc/{}/limits", child.id()))?;
        let memory = limits
            .lines()
            .find(|l| l.starts_with("Max address space"))
            .unwrap();
        assert!(memory.contains("536870912"));
        let cpu = limits
            .lines()
            .find(|l| l.starts_with("Max cpu time"))
            .unwrap();
        assert!(cpu.contains("120"));
        sidenote_sync::containment::kill_group(child.id());
        assert!(!child.wait()?.success());
        Ok(())
    }
}

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
// Subprocess execution stays unavailable until Unix containment is qualified.
pub enum ProcessJob {}
impl ProcessJob {
    pub fn attach(_child: &std::process::Child) -> io::Result<Self> {
        Err(super::unavailable())
    }
}
impl Drop for ProcessJob {
    fn drop(&mut self) {
        match *self {}
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

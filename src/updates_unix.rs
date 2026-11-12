use std::{fs::File, io};
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub fn activity() -> io::Result<Option<File>> {
    Ok(None)
}
pub fn listener_lock() -> io::Result<Option<File>> {
    crate::core::store_io::lock(&crate::core::state_dir().join("local/service.lock")).map(Some)
}
pub fn listener_stopping() -> bool {
    crate::service::stopping()
}
pub fn cli(_args: &[std::ffi::OsString]) -> i32 {
    eprintln!("{}", crate::platform::UNAVAILABLE);
    2
}

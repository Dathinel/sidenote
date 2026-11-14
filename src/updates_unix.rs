pub use crate::update_state::{CheckRequest, ListenerStatus, Phase, Status};
use std::{fs::File, io};
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub fn activity() -> io::Result<Option<File>> {
    Ok(None)
}
pub fn listener_lock() -> io::Result<Option<File>> {
    crate::core::store_io::lock(&crate::core::state_dir().join("local/service.lock")).map(Some)
}
pub fn listener_stopping() -> bool {
    listener_stop_source().is_some()
}
pub(crate) fn listener_stop_source() -> Option<&'static str> {
    crate::service::stopping().then_some("service-stop")
}
pub fn cli(_args: &[std::ffi::OsString]) -> i32 {
    eprintln!("{}", crate::platform::UNAVAILABLE);
    2
}

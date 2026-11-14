//! Linux packaging and Hex installation are not available in the launch layer.
pub use crate::hex_install_state::{Installation, State};
pub const DOWNLOAD_PAGE: &str = "https://github.com/anomalyco/hex/releases/latest";
pub fn detect() -> Installation {
    Installation::Incomplete
}
pub fn install_after_click(mut report: impl FnMut(State)) {
    report(State::Failed("Hex installation is unavailable on Linux."));
}

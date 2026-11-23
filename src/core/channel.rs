#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    Stable,
    Nightly,
}

pub const CHANNEL: Channel = if cfg!(feature = "nightly") {
    Channel::Nightly
} else {
    Channel::Stable
};

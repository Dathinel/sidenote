#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Channel {
    Stable,
    Dev,
}

pub const CHANNEL: Channel = if cfg!(feature = "dev") {
    Channel::Dev
} else {
    Channel::Stable
};

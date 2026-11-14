//! Shared installation status shown by the desktop.
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installation {
    Missing,
    Installed { version: String, exe: PathBuf },
    Incomplete,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum State {
    #[default]
    Idle,
    Checking,
    Downloading {
        bytes: u64,
        total: u64,
    },
    Verifying,
    Installing,
    Complete,
    Failed(&'static str),
}
impl State {
    pub fn busy(&self) -> bool {
        matches!(
            self,
            Self::Checking | Self::Downloading { .. } | Self::Verifying | Self::Installing
        )
    }

    /// The UI calls this only from Install Hex. Duplicate clicks cannot start another worker.
    pub fn begin(&mut self, installation: &Installation) -> bool {
        if self.busy() || *installation != Installation::Missing || *self == Self::Complete {
            return false;
        }
        *self = Self::Checking;
        true
    }

    pub fn message(&self) -> String {
        match self {
            Self::Idle => {
                "Sidenote needs Hex to record and transcribe. Install Hex for this Windows account."
                    .into()
            }
            Self::Checking => "Checking the latest Hex release.".into(),
            Self::Downloading { bytes, total } => format!(
                "Downloading Hex: {} of {} MB ({}%).",
                bytes / 1_000_000,
                total.div_ceil(1_000_000),
                bytes.saturating_mul(100) / (*total).max(1),
            ),
            Self::Verifying => "Checking the download and Anomaly Innovations signature.".into(),
            Self::Installing => "Installing Hex for this Windows account.".into(),
            Self::Complete => {
                "Hex is installed. Open Hex to finish its setup. Sidenote will connect here.".into()
            }
            Self::Failed(message) => (*message).into(),
        }
    }
}

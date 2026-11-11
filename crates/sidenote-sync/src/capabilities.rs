//! Authenticated group requirements are monotonic and durable per room.
use crate::{
    identity::{protect, unprotect},
    status::{Failure, Reason},
    store::Store,
};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub schema: u32,
    pub features: u32,
}
impl Default for Capabilities {
    fn default() -> Self {
        Self::CURRENT
    }
}
impl Capabilities {
    pub const CURRENT: Self = Self {
        schema: 1,
        features: 1,
    };
    pub fn supports(self, required: Self) -> bool {
        self.schema >= required.schema && self.features >= required.features
    }
    fn max(self, other: Self) -> Self {
        Self {
            schema: self.schema.max(other.schema),
            features: self.features.max(other.features),
        }
    }
}
pub(crate) struct Policy {
    slot: String,
    supported: Capabilities,
    required: Capabilities,
}
impl Store {
    pub fn configure_capabilities(&mut self, room: &str, supported: Capabilities) -> Result<()> {
        ensure!(
            room.len() == 64 && room.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid room"
        );
        ensure!(
            supported.schema > 0 && supported.features > 0,
            "invalid capabilities"
        );
        let slot = format!("capabilities:{room}");
        let prior = self
            .protected(&slot)?
            .map(|b| -> Result<Capabilities> { Ok(serde_json::from_slice(&unprotect(&b)?)?) })
            .transpose()?
            .unwrap_or(supported);
        let required = prior.max(supported);
        self.put_protected(&slot, &protect(&serde_json::to_vec(&required)?)?)?;
        self.capabilities = Some(Policy {
            slot,
            supported,
            required,
        });
        Ok(())
    }
    pub fn requirements(&self) -> Option<Capabilities> {
        self.capabilities.as_ref().map(|p| p.required)
    }
    pub fn check_compatible(&self) -> Result<()> {
        if let Some(p) = &self.capabilities {
            ensure!(
                p.supported.supports(p.required),
                Failure(Reason::Incompatible)
            );
        }
        Ok(())
    }
    pub(crate) fn observe_capabilities(&mut self, remote: Capabilities) -> Result<()> {
        ensure!(
            remote.schema > 0 && remote.features > 0,
            Failure(Reason::Protocol)
        );
        ensure!(
            remote.schema <= Capabilities::CURRENT.schema + 1
                && remote.features <= Capabilities::CURRENT.features + 1,
            Failure(Reason::PeerBadData)
        );
        if let Some(p) = &self.capabilities {
            let required = p.required.max(remote);
            let slot = p.slot.clone();
            self.capabilities.as_mut().expect("policy").required = required;
            self.put_protected(&slot, &protect(&serde_json::to_vec(&required)?)?)?;
        }
        self.check_compatible()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unbounded_remote_requirements_do_not_poison_the_durable_policy() -> Result<()> {
        let d = tempfile::tempdir()?;
        let path = d.path().join("replica.sqlite");
        let room = "a".repeat(64);
        let mut s = Store::open(&path, b"test")?;
        s.configure_capabilities(&room, Capabilities::CURRENT)?;
        for remote in [
            Capabilities {
                schema: u32::MAX,
                features: 1,
            },
            Capabilities {
                schema: 1,
                features: u32::MAX,
            },
        ] {
            assert!(s.observe_capabilities(remote).is_err());
            s.check_compatible()?;
            assert_eq!(s.requirements(), Some(Capabilities::CURRENT));
        }
        drop(s);
        let mut s = Store::open(&path, b"test")?;
        s.configure_capabilities(&room, Capabilities::CURRENT)?;
        s.check_compatible()?;
        Ok(())
    }
    #[test]
    fn group_requirement_survives_downgrade_and_cannot_be_lowered() -> Result<()> {
        let d = tempfile::tempdir()?;
        let path = d.path().join("replica.sqlite");
        let room = "a".repeat(64);
        {
            let mut s = Store::open(&path, b"test")?;
            s.configure_capabilities(&room, Capabilities::CURRENT)?;
            assert!(
                s.observe_capabilities(Capabilities {
                    schema: 2,
                    features: 1
                })
                .is_err()
            );
            assert!(s.observe_capabilities(Capabilities::CURRENT).is_err());
            assert!(s.edit("tasks", |_| Ok(())).is_err());
        }
        let mut s = Store::open(&path, b"test")?;
        s.configure_capabilities(&room, Capabilities::CURRENT)?;
        assert!(s.check_compatible().is_err());
        s.configure_capabilities(
            &room,
            Capabilities {
                schema: 2,
                features: 1,
            },
        )?;
        s.check_compatible()?;
        assert!(
            s.observe_capabilities(Capabilities {
                schema: 2,
                features: 2
            })
            .is_err()
        );
        s.configure_capabilities(&"b".repeat(64), Capabilities::CURRENT)?;
        s.check_compatible()?;
        Ok(())
    }
}

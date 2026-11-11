//! Fixed binary framing bounds lengths before allocation. No JSON byte arrays.
use crate::{
    identity::Enrollment,
    preflight::{MAX_BULK, Reader},
    status::{Failure, Reason},
    store::{INVENTORY_PAGE, valid_domain},
};
use anyhow::{Result, ensure};
use automerge::ChangeHash;
pub const PING: u8 = 4;
pub const PONG: u8 = 5;
pub(crate) enum Inner {
    Capabilities(crate::capabilities::Capabilities),
    Inventory {
        done: bool,
        domains: Vec<(String, Vec<ChangeHash>)>,
    },
    Sync {
        domain: String,
        bytes: Vec<u8>,
    },
    Membership(Vec<Enrollment>),
    Ping([u8; 32]),
    Pong([u8; 32]),
}
fn text(out: &mut Vec<u8>, value: &str) -> Result<()> {
    ensure!(valid_domain(value), "domain");
    out.extend_from_slice(&(value.len() as u16).to_be_bytes());
    out.extend_from_slice(value.as_bytes());
    Ok(())
}
fn domain(r: &mut Reader<'_>) -> Result<String> {
    let len = u16::from_be_bytes(r.take(2)?.try_into()?) as usize;
    ensure!(len <= 136, "domain length");
    let value = std::str::from_utf8(r.take(len)?)?;
    ensure!(valid_domain(value), "domain");
    Ok(value.to_owned())
}
impl Inner {
    pub fn encode(self) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        match self {
            Self::Capabilities(c) => {
                out.push(6);
                out.extend_from_slice(&c.schema.to_be_bytes());
                out.extend_from_slice(&c.features.to_be_bytes());
            }
            Self::Inventory { done, domains } => {
                ensure!(domains.len() <= INVENTORY_PAGE, "page cap");
                out.extend_from_slice(&[1, u8::from(done)]);
                out.extend_from_slice(&(domains.len() as u16).to_be_bytes());
                for (id, heads) in domains {
                    text(&mut out, &id)?;
                    ensure!(heads.len() <= 1024, "heads cap");
                    out.extend_from_slice(&(heads.len() as u16).to_be_bytes());
                    for h in heads {
                        out.extend_from_slice(h.as_ref());
                    }
                }
            }
            Self::Sync { domain, bytes } => {
                out.push(2);
                text(&mut out, &domain)?;
                out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
                out.extend_from_slice(&bytes);
            }
            Self::Membership(proofs) => {
                ensure!(proofs.len() <= 16, "membership cap");
                out.push(3);
                out.extend_from_slice(
                    &serde_json::to_vec(&proofs).map_err(|_| Failure(Reason::Protocol))?,
                );
            }
            Self::Ping(value) => {
                out.push(PING);
                out.extend_from_slice(&value);
            }
            Self::Pong(value) => {
                out.push(PONG);
                out.extend_from_slice(&value);
            }
        }
        ensure!(out.len() <= MAX_BULK, "bulk cap");
        Ok(out)
    }
    pub fn decode(mut bytes: Vec<u8>) -> Result<Self> {
        ensure!(bytes.len() <= MAX_BULK, "bulk cap");
        let mut r = Reader::new(&bytes);
        let message = match r.byte()? {
            1 => {
                let done = match r.byte()? {
                    0 => false,
                    1 => true,
                    _ => anyhow::bail!("page flags"),
                };
                let count = u16::from_be_bytes(r.take(2)?.try_into()?) as usize;
                ensure!(count <= INVENTORY_PAGE, "page cap");
                let mut domains = Vec::with_capacity(count);
                for _ in 0..count {
                    let id = domain(&mut r)?;
                    let n = u16::from_be_bytes(r.take(2)?.try_into()?) as usize;
                    ensure!(n <= 1024, "heads cap");
                    let mut heads = Vec::with_capacity(n);
                    for _ in 0..n {
                        heads.push(ChangeHash(r.take(32)?.try_into()?));
                    }
                    domains.push((id, heads));
                }
                Self::Inventory { done, domains }
            }
            2 => {
                let domain = domain(&mut r)?;
                let len = u32::from_be_bytes(r.take(4)?.try_into()?) as usize;
                ensure!(r.rest().len() == len, "sync length");
                let start = r.offset;
                bytes.copy_within(start.., 0);
                bytes.truncate(len);
                return Ok(Self::Sync { domain, bytes });
            }
            3 => {
                ensure!(r.rest().len() <= 16 * 1024, "membership bytes cap");
                let proofs: Vec<Enrollment> =
                    serde_json::from_slice(r.rest()).map_err(|_| Failure(Reason::Protocol))?;
                ensure!(proofs.len() <= 16, "membership cap");
                r.take(r.rest().len())?;
                Self::Membership(proofs)
            }
            6 => Self::Capabilities(crate::capabilities::Capabilities {
                schema: u32::from_be_bytes(r.take(4)?.try_into()?),
                features: u32::from_be_bytes(r.take(4)?.try_into()?),
            }),
            PING => Self::Ping(r.take(32)?.try_into()?),
            PONG => Self::Pong(r.take(32)?.try_into()?),
            _ => anyhow::bail!("inner version"),
        };
        ensure!(r.rest().is_empty(), "trailing binary input");
        Ok(message)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn f8_wire_caps_and_trailing_bytes_are_rejected() -> Result<()> {
        assert_eq!(
            Inner::decode(vec![0; MAX_BULK + 1])
                .err()
                .expect("wire cap")
                .to_string(),
            "bulk cap"
        );
        let mut ping = Inner::Ping([0; 32]).encode()?;
        ping.push(0);
        assert_eq!(
            Inner::decode(ping)
                .err()
                .expect("trailing byte")
                .to_string(),
            "trailing binary input"
        );
        let mut page = vec![1, 1];
        page.extend_from_slice(&129u16.to_be_bytes());
        assert_eq!(
            Inner::decode(page).err().expect("page cap").to_string(),
            "page cap"
        );
        let mut sync = Inner::Sync {
            domain: "tasks".into(),
            bytes: vec![1],
        }
        .encode()?;
        sync.push(0);
        assert_eq!(
            Inner::decode(sync)
                .err()
                .expect("trailing sync byte")
                .to_string(),
            "sync length"
        );
        Ok(())
    }
    #[test]
    fn f7_parsed_values_never_escape_as_serde_errors() {
        let mut bytes = vec![3];
        bytes.extend_from_slice(br#"[{"sponsor":"private-marker"}]"#);
        let error = Inner::decode(bytes).err().expect("invalid membership");
        assert_eq!(
            error.downcast_ref::<Failure>().map(|f| f.0),
            Some(Reason::Protocol)
        );
        assert!(!error.to_string().contains("private-marker"));
    }
    #[test]
    fn bulk_encoding_is_binary_and_length_lies_are_rejected() -> Result<()> {
        let payload = vec![255; 9 * 1024 * 1024];
        let encoded = Inner::Sync {
            domain: "tasks".into(),
            bytes: payload.clone(),
        }
        .encode()?;
        assert_eq!(encoded.len(), payload.len() + 12);
        assert!(matches!(Inner::decode(encoded)?,Inner::Sync {bytes,..} if bytes==payload));
        assert!(
            Inner::decode(vec![
                2, 0, 5, b't', b'a', b's', b'k', b's', 255, 255, 255, 255
            ])
            .is_err()
        );
        Ok(())
    }
}

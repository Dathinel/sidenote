//! Signed envelopes bind Noise handshakes/traffic to pinned Ed25519 identities.
use crate::identity::{Group, Identity, verify};
use anyhow::{Context, Result, ensure};

pub(crate) const HEADER: usize = 98;
pub(crate) const SIGNATURE: usize = 64;
pub(crate) const MAX_MESSAGE: usize = crate::preflight::MAX_BULK;
pub(crate) const CHUNK: usize = 32 * 1024;

pub(crate) fn handshake(
    group: &Group,
    local: [u8; 32],
    peer: [u8; 32],
    initiator: bool,
    freshness: &[u8; 64],
) -> Result<snow::HandshakeState> {
    let mut prologue = b"sidenote-noise-v1".to_vec();
    prologue.extend_from_slice(&group.id);
    prologue.extend_from_slice(freshness);
    let (a, b) = if local < peer {
        (local, peer)
    } else {
        (peer, local)
    };
    prologue.extend_from_slice(&a);
    prologue.extend_from_slice(&b);
    let builder = snow::Builder::new("Noise_NNpsk0_25519_ChaChaPoly_SHA256".parse()?)
        .psk(0, group.key())?
        .prologue(&prologue)?;
    Ok(if initiator {
        builder.build_initiator()?
    } else {
        builder.build_responder()?
    })
}

pub(crate) fn seal(
    identity: &Identity,
    group: &Group,
    peer: [u8; 32],
    kind: u8,
    session: [u8; 32],
    payload: &[u8],
) -> Vec<u8> {
    let mut frame = vec![1, kind];
    frame.extend_from_slice(&identity.public());
    frame.extend_from_slice(&peer);
    frame.extend_from_slice(&session);
    frame.extend_from_slice(payload);
    let mut signed = group.id.to_vec();
    signed.extend_from_slice(&frame);
    frame.extend_from_slice(&identity.sign(&signed));
    frame
}

pub(crate) struct Opened<'a> {
    pub kind: u8,
    pub session: [u8; 32],
    pub payload: &'a [u8],
}
pub(crate) fn open<'a>(identity: &Identity, group: &Group, frame: &'a [u8]) -> Result<Opened<'a>> {
    ensure!(
        frame.len() >= HEADER + SIGNATURE && frame[0] == 1 && (1..=5).contains(&frame[1]),
        "invalid signed envelope"
    );
    let sender = frame[2..34].try_into()?;
    ensure!(
        group.members().contains(&sender) && sender != identity.public(),
        "unpinned sender"
    );
    ensure!(frame[34..66] == identity.public(), "wrong recipient");
    let mut signed = group.id.to_vec();
    signed.extend_from_slice(&frame[..frame.len() - SIGNATURE]);
    verify(&sender, &signed, &frame[frame.len() - SIGNATURE..])?;
    Ok(Opened {
        kind: frame[1],
        session: frame[66..98].try_into()?,
        payload: &frame[HEADER..frame.len() - SIGNATURE],
    })
}

pub(crate) struct Cipher {
    pub state: snow::TransportState,
    pub session: [u8; 32],
    assembly: Vec<u8>,
    total: usize,
}
impl Cipher {
    pub fn pending_bytes(&self) -> usize {
        self.assembly.len()
    }
    pub fn encrypt_fragment(&mut self, message: &[u8], offset: usize) -> Result<(Vec<u8>, usize)> {
        ensure!(
            !message.is_empty() && message.len() <= MAX_MESSAGE && offset < message.len(),
            "bulk fragment"
        );
        let end = (offset + CHUNK).min(message.len());
        let mut plain = Vec::with_capacity(end - offset + 8);
        plain.extend_from_slice(&(message.len() as u32).to_be_bytes());
        plain.extend_from_slice(&(offset as u32).to_be_bytes());
        plain.extend_from_slice(&message[offset..end]);
        let mut encrypted = vec![0; plain.len() + 16];
        let len = self.state.write_message(&plain, &mut encrypted)?;
        encrypted.truncate(len);
        Ok((encrypted, end))
    }
    pub fn encrypt_control(&mut self, message: &[u8]) -> Result<Vec<u8>> {
        ensure!(
            message.len() == 33 && [crate::wire::PING, crate::wire::PONG].contains(&message[0]),
            "noise control"
        );
        let mut plain = vec![0; 8];
        plain.extend_from_slice(message);
        let mut out = vec![0; plain.len() + 16];
        let len = self.state.write_message(&plain, &mut out)?;
        out.truncate(len);
        Ok(out)
    }
    pub fn from_handshake(state: snow::HandshakeState) -> Result<Self> {
        let session = state
            .get_handshake_hash()
            .try_into()
            .context("invalid handshake hash")?;
        Ok(Self {
            state: state.into_transport_mode()?,
            session,
            assembly: Vec::new(),
            total: 0,
        })
    }
    pub fn decrypt(&mut self, encrypted: &[u8]) -> Result<Option<Vec<u8>>> {
        let mut clear = vec![0; encrypted.len()];
        let len = self
            .state
            .read_message(encrypted, &mut clear)
            .map_err(|_| crate::status::Failure(crate::status::Reason::Transport))?;
        ensure!(len > 8, "invalid fragment");
        let total = u32::from_be_bytes(clear[..4].try_into()?) as usize;
        let offset = u32::from_be_bytes(clear[4..8].try_into()?) as usize;
        if total == 0 {
            ensure!(
                offset == 0
                    && len == 41
                    && [crate::wire::PING, crate::wire::PONG].contains(&clear[8]),
                "invalid control fragment"
            );
            return Ok(Some(clear[8..len].to_vec()));
        }
        ensure!(
            total <= MAX_MESSAGE
                && total > 0
                && offset == self.assembly.len()
                && offset + len - 8 <= total,
            "invalid fragment bounds"
        );
        if offset == 0 {
            self.total = total;
        } else {
            ensure!(total == self.total, "fragment length changed");
        }
        self.assembly.extend_from_slice(&clear[8..len]);
        if self.assembly.len() == total {
            self.total = 0;
            Ok(Some(std::mem::take(&mut self.assembly)))
        } else {
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{identity::Identity, store::Store};
    #[test]
    fn authenticated_cipher_rejects_tamper_replay_and_old_session() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let a = Identity::load_or_create(&Store::open(&dir.path().join("a"), b"A")?)?;
        let b = Identity::load_or_create(&Store::open(&dir.path().join("b"), b"B")?)?;
        let mut group = Group::new(&a);
        group.enroll(&a, b.public());
        let establish = || -> Result<(Cipher, Cipher)> {
            let mut freshness = [0; 64];
            freshness[..32].copy_from_slice(&crate::identity::random32());
            freshness[32..].copy_from_slice(&crate::identity::random32());
            let mut alice = handshake(&group, a.public(), b.public(), true, &freshness)?;
            let mut bob = handshake(&group, b.public(), a.public(), false, &freshness)?;
            let mut buf = [0; 1024];
            let mut out = [0; 1024];
            let len = alice.write_message(&[], &mut buf)?;
            bob.read_message(&buf[..len], &mut out)?;
            let len = bob.write_message(&[], &mut buf)?;
            alice.read_message(&buf[..len], &mut out)?;
            Ok((Cipher::from_handshake(alice)?, Cipher::from_handshake(bob)?))
        };
        let (mut sender, mut receiver) = establish()?;
        let data = vec![b'x'; 9 * 1024 * 1024 + 4];
        let mut encrypted = Vec::new();
        let mut offset = 0;
        while offset < data.len() {
            let (frame, end) = sender.encrypt_fragment(&data, offset)?;
            encrypted.push(frame);
            offset = end;
        }
        let frame = seal(&a, &group, b.public(), 3, sender.session, &encrypted[0]);
        assert!(open(&b, &group, &frame).is_ok());
        let mut bad = frame.clone();
        bad[HEADER] ^= 1;
        assert!(open(&b, &group, &bad).is_err());
        let mut result = None;
        for e in &encrypted {
            result = receiver.decrypt(e)?;
        }
        assert_eq!(result, Some(data));
        let interleaved = vec![b'z'; CHUNK + 1];
        let (first, end) = sender.encrypt_fragment(&interleaved, 0)?;
        assert_eq!(receiver.decrypt(&first)?, None);
        let ping = crate::wire::Inner::Ping([4; 32]).encode()?;
        assert_eq!(
            receiver.decrypt(&sender.encrypt_control(&ping)?)?,
            Some(ping)
        );
        let (last, _) = sender.encrypt_fragment(&interleaved, end)?;
        assert_eq!(receiver.decrypt(&last)?, Some(interleaved));
        assert!(receiver.decrypt(&encrypted[0]).is_err());
        let (_, mut fresh) = establish()?;
        assert_ne!(fresh.session, sender.session);
        assert!(fresh.decrypt(&encrypted[0]).is_err());
        Ok(())
    }
    #[test]
    fn qualification_three_member_rotation_rejects_old_key_and_old_group_frames() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let a = Identity::load_or_create(&Store::open(&dir.path().join("a"), b"a")?)?;
        let b = Identity::load_or_create(&Store::open(&dir.path().join("b"), b"b")?)?;
        let c = Identity::load_or_create(&Store::open(&dir.path().join("c"), b"c")?)?;
        let mut old = Group::new(&a);
        old.enroll(&a, b.public());
        old.enroll(&a, c.public());
        let mut fresh = Group::new(&a);
        fresh.enroll(&a, b.public());
        assert!(old.key() != fresh.key(), "rotation reused the old key");
        assert_eq!(old.members().len(), 3);
        assert_eq!(fresh.members().len(), 2);
        let nonce = [8; 64];
        let mut sender = handshake(&fresh, a.public(), b.public(), true, &nonce)?;
        let mut removed = handshake(&old, b.public(), a.public(), false, &nonce)?;
        let mut bytes = [0; 1024];
        let mut clear = [0; 1024];
        let n = sender.write_message(&[], &mut bytes)?;
        assert!(
            removed.read_message(&bytes[..n], &mut clear).is_err(),
            "old group key opened the new handshake"
        );
        let frame = seal(
            &c,
            &old,
            a.public(),
            3,
            [0; 32],
            b"Synthetic old-room traffic",
        );
        assert!(open(&a, &fresh, &frame).is_err());
        let mut survivor = handshake(&fresh, b.public(), a.public(), false, &nonce)?;
        survivor.read_message(&bytes[..n], &mut clear)?;
        let n = survivor.write_message(&[], &mut bytes)?;
        sender.read_message(&bytes[..n], &mut clear)?;
        let (mut sender, mut survivor) = (
            Cipher::from_handshake(sender)?,
            Cipher::from_handshake(survivor)?,
        );
        let content = b"Synthetic new content only for the re-paired survivor";
        let (encrypted, end) = sender.encrypt_fragment(content, 0)?;
        assert_eq!(end, content.len());
        assert_eq!(survivor.decrypt(&encrypted)?, Some(content.to_vec()));
        Ok(())
    }
}

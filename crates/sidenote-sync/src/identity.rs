use crate::{
    status::{Failure, Reason},
    store::Store,
};
use anyhow::{Context, Result, ensure};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand::{RngCore, rngs::OsRng};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::{Zeroize, Zeroizing};

pub struct Identity(SigningKey);

impl Identity {
    pub fn load_or_create(store: &Store) -> Result<Self> {
        if store.protected("identity")?.is_none() {
            let secret = Zeroizing::new(SigningKey::generate(&mut OsRng).to_bytes());
            store.insert_protected("identity", &protect(secret.as_ref())?)?;
        }
        let bytes = unprotect(&store.protected("identity")?.context("identity missing")?)?;
        let secret: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .context("invalid identity length")?;
        let key = Self(SigningKey::from_bytes(&secret));
        let mut secret = secret;
        secret.zeroize();
        Ok(key)
    }

    pub fn public(&self) -> [u8; 32] {
        self.0.verifying_key().to_bytes()
    }
    pub fn sign(&self, bytes: &[u8]) -> Vec<u8> {
        self.0.sign(bytes).to_bytes().to_vec()
    }
}

pub fn verify(public: &[u8; 32], bytes: &[u8], signature: &[u8]) -> Result<()> {
    VerifyingKey::from_bytes(public)?.verify_strict(bytes, &Signature::from_slice(signature)?)?;
    Ok(())
}

pub fn hash(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

pub fn random32() -> [u8; 32] {
    let mut bytes = [0; 32];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Enrollment {
    pub sponsor: [u8; 32],
    pub member: [u8; 32],
    pub signature: Vec<u8>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: [u8; 32],
    #[serde(with = "secret_serde")]
    key: Zeroizing<[u8; 32]>,
    pub enrollments: Vec<Enrollment>,
}

mod secret_serde {
    use super::*;
    pub fn serialize<S: serde::Serializer>(
        key: &Zeroizing<[u8; 32]>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        key.as_ref().serialize(serializer)
    }
    pub fn deserialize<'de, D: serde::Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Zeroizing<[u8; 32]>, D::Error> {
        Ok(Zeroizing::new(<[u8; 32]>::deserialize(deserializer)?))
    }
}

impl Group {
    pub fn new(identity: &Identity) -> Self {
        let mut group = Self {
            id: random32(),
            key: Zeroizing::new(random32()),
            enrollments: Vec::new(),
        };
        group.enroll(identity, identity.public());
        group
    }
    pub fn room(&self) -> String {
        hex::encode(hash(&self.id))
    }
    pub(crate) fn key(&self) -> &[u8; 32] {
        &self.key
    }
    pub fn members(&self) -> Vec<[u8; 32]> {
        self.enrollments.iter().map(|e| e.member).collect()
    }
    pub fn enroll(&mut self, sponsor: &Identity, member: [u8; 32]) {
        if !self.members().contains(&member) {
            self.enrollments.push(Enrollment {
                sponsor: sponsor.public(),
                member,
                signature: sponsor.sign(&enrollment_bytes(&self.room(), &member)),
            });
        }
    }
    pub fn validate(&self, local: [u8; 32]) -> Result<()> {
        ensure!(
            !self.enrollments.is_empty() && self.enrollments.len() <= 16,
            "invalid group size"
        );
        let first = &self.enrollments[0];
        ensure!(first.member == first.sponsor, "invalid genesis");
        let mut known = Vec::new();
        for e in &self.enrollments {
            ensure!(
                known.is_empty() || known.contains(&e.sponsor),
                "unknown sponsor"
            );
            verify(
                &e.sponsor,
                &enrollment_bytes(&self.room(), &e.member),
                &e.signature,
            )?;
            ensure!(!known.contains(&e.member), "duplicate member");
            known.push(e.member);
        }
        ensure!(known.contains(&local), "local key absent from group");
        Ok(())
    }
    pub(crate) fn same_key(&self, other: &Self) -> bool {
        use subtle::ConstantTimeEq;
        bool::from(self.key.as_ref().ct_eq(other.key.as_ref()))
    }
    pub fn load(store: &Store, room: &str) -> Result<Self> {
        store.check_group_active(room)?;
        ensure!(
            room.len() == 64 && hex::decode(room).is_ok(),
            "invalid room"
        );
        let bytes = unprotect(
            &store
                .protected(&format!("group:{room}"))?
                .context("no group")?,
        )?;
        Ok(serde_json::from_slice(&bytes).map_err(|_| Failure(Reason::CorruptDocument))?)
    }
}

impl Drop for Group {
    fn drop(&mut self) {
        self.key.zeroize();
    }
}

pub fn enrollment_bytes(room: &str, member: &[u8; 32]) -> Vec<u8> {
    format!("sidenote-enroll-v1:{room}:{}", hex::encode(member)).into_bytes()
}

pub fn auth_bytes(room: &str, role: &str, nonce: &str) -> Vec<u8> {
    format!("sidenote-auth-v1:{room}:{role}:{nonce}").into_bytes()
}

#[cfg(windows)]
fn dpapi(bytes: &[u8], encrypt: bool) -> Result<Zeroizing<Vec<u8>>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    ensure!(bytes.len() <= u32::MAX as usize, "DPAPI input too large");
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // DPAPI allocates output with LocalAlloc; copy, erase, and free even for plaintext.
    let success = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    ensure!(success != 0, Failure(Reason::LocalStore));
    let result = unsafe {
        let slice = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let result = Zeroizing::new(slice.to_vec());
        slice.zeroize();
        LocalFree(output.pbData.cast());
        result
    };
    Ok(result)
}

pub fn protect(bytes: &[u8]) -> Result<Vec<u8>> {
    #[cfg(windows)]
    {
        Ok(dpapi(bytes, true)?.to_vec())
    }
    #[cfg(target_os = "linux")]
    {
        crate::linux_secret::protect(bytes).context(Failure(Reason::LocalStore))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = bytes;
        anyhow::bail!("UNAVAILABLE_ON_PLATFORM")
    }
}
pub fn unprotect(bytes: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    #[cfg(windows)]
    {
        dpapi(bytes, false)
    }
    #[cfg(target_os = "linux")]
    {
        crate::linux_secret::unprotect(bytes).context(Failure(Reason::LocalStore))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = bytes;
        anyhow::bail!("UNAVAILABLE_ON_PLATFORM")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Device;
    #[test]
    fn c7_group_id_key_and_genesis_cannot_be_replaced() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let a = Device::open(&dir.path().join("a.sqlite"))?;
        let other = Device::open(&dir.path().join("other.sqlite"))?;
        let original = Group::new(&a.identity);
        a.merge_group(&original)?;
        let mut wrong_id = original.clone();
        wrong_id.id[0] ^= 1;
        assert!(a.merge_group(&wrong_id).is_err());
        let mut wrong_key = original.clone();
        wrong_key.key[0] ^= 1;
        assert!(a.merge_group(&wrong_key).is_err());
        let mut wrong_genesis = original.clone();
        wrong_genesis.enrollments.clear();
        wrong_genesis.enroll(&other.identity, other.identity.public());
        assert!(a.merge_group(&wrong_genesis).is_err());
        let loaded = a.store(|store| Group::load(store, &original.room()))?;
        assert!(loaded.same_key(&original));
        assert_eq!(loaded.members(), original.members());
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub fn cancel_pending_initialization() {
    crate::linux_secret::cancel();
}

#[cfg(target_os = "linux")]
pub const SECRET_INIT_ERROR: &str = crate::linux_secret::UNAVAILABLE;

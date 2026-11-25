//! Explicit passphrase vault for sessions without an unlocked Secret Service.
use super::SecretStore;
use chacha20poly1305::{
    aead::{Aead, Payload},
    KeyInit, XChaCha20Poly1305, XNonce,
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::{self, Read, Write},
    path::PathBuf,
};
use zeroize::{Zeroize, Zeroizing};
const AAD: &[u8] = b"SaveSync encrypted credential vault v1";
const LIMIT: usize = 4 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
struct Envelope {
    format: u32,
    salt: Vec<u8>,
    nonce: Vec<u8>,
    ciphertext: Vec<u8>,
}
#[derive(Default, Serialize, Deserialize)]
struct Secrets(BTreeMap<String, String>);
impl Drop for Secrets {
    fn drop(&mut self) {
        for value in self.0.values_mut() {
            value.zeroize();
        }
    }
}
pub(super) struct FileStore {
    path: PathBuf,
    passphrase: Zeroizing<String>,
}
fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "encrypted credential vault is locked, corrupted or has a different passphrase",
    )
}
impl FileStore {
    pub fn from_environment() -> io::Result<Option<Self>> {
        let passphrase = match std::env::var("SAVESYNC_VAULT_PASSPHRASE") {
            Ok(value) => value,
            Err(std::env::VarError::NotPresent) => return Ok(None),
            Err(_) => return Err(denied()),
        };
        let directory =
            crate::config::data_dir(crate::paths::Platform::current()).ok_or_else(denied)?;
        Self::new(directory.join("credentials.encrypted.json"), passphrase).map(Some)
    }
    fn new(path: PathBuf, passphrase: String) -> io::Result<Self> {
        if passphrase.chars().count() < 12 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "vault passphrase must contain at least 12 characters",
            ));
        }
        Ok(Self {
            path,
            passphrase: Zeroizing::new(passphrase),
        })
    }
    fn key(&self, salt: &[u8]) -> io::Result<Zeroizing<[u8; 32]>> {
        let mut key = Zeroizing::new([0; 32]);
        let params = argon2::Params::new(19_456, 2, 1, Some(32)).map_err(|_| denied())?;
        argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params)
            .hash_password_into(self.passphrase.as_bytes(), salt, key.as_mut())
            .map_err(|_| denied())?;
        Ok(key)
    }
    fn guard(&self) -> io::Result<std::fs::File> {
        let parent = self.path.parent().ok_or_else(denied)?;
        std::fs::create_dir_all(parent)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(self.path.with_extension("lock"))?;
        fs2::FileExt::lock_exclusive(&file)?;
        Ok(file)
    }
    fn read(&self) -> io::Result<(Secrets, Vec<u8>)> {
        let file = match std::fs::File::open(&self.path) {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let mut salt = vec![0; 16];
                OsRng.fill_bytes(&mut salt);
                return Ok((Secrets::default(), salt));
            }
            Err(error) => return Err(error),
        };
        let mut bytes = Vec::new();
        file.take((LIMIT + 1) as u64).read_to_end(&mut bytes)?;
        if bytes.len() > LIMIT {
            return Err(denied());
        }
        let envelope: Envelope = serde_json::from_slice(&bytes).map_err(|_| denied())?;
        if envelope.format != 1 || envelope.salt.len() != 16 || envelope.nonce.len() != 24 {
            return Err(denied());
        }
        let key = self.key(&envelope.salt)?;
        let plaintext = Zeroizing::new(
            XChaCha20Poly1305::new_from_slice(key.as_ref())
                .map_err(|_| denied())?
                .decrypt(
                    XNonce::from_slice(&envelope.nonce),
                    Payload {
                        msg: &envelope.ciphertext,
                        aad: AAD,
                    },
                )
                .map_err(|_| denied())?,
        );
        let secrets = serde_json::from_slice(&plaintext).map_err(|_| denied())?;
        Ok((secrets, envelope.salt))
    }
    fn write(&self, secrets: &Secrets, salt: Vec<u8>) -> io::Result<()> {
        let key = self.key(&salt)?;
        let mut nonce = vec![0; 24];
        OsRng.fill_bytes(&mut nonce);
        let plaintext = Zeroizing::new(serde_json::to_vec(secrets).map_err(io::Error::other)?);
        let ciphertext = XChaCha20Poly1305::new_from_slice(key.as_ref())
            .map_err(|_| denied())?
            .encrypt(
                XNonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: AAD,
                },
            )
            .map_err(|_| denied())?;
        let bytes = serde_json::to_vec(&Envelope {
            format: 1,
            salt,
            nonce,
            ciphertext,
        })
        .map_err(io::Error::other)?;
        if bytes.len() > LIMIT {
            return Err(io::Error::other("credential vault size limit exceeded"));
        }
        let temporary = self.path.with_extension("tmp");
        let mut options = std::fs::OpenOptions::new();
        options.create(true).truncate(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(temporary, &self.path)?;
        #[cfg(unix)]
        {
            std::fs::File::open(self.path.parent().ok_or_else(denied)?)?.sync_all()?;
        }
        Ok(())
    }
}
impl SecretStore for FileStore {
    fn get(&self, key: &str) -> io::Result<String> {
        let _guard = self.guard()?;
        self.read()?
            .0
             .0
            .get(key)
            .cloned()
            .ok_or_else(|| io::ErrorKind::NotFound.into())
    }
    fn put(&self, key: &str, value: &str) -> io::Result<()> {
        let _guard = self.guard()?;
        let (mut secrets, salt) = self.read()?;
        secrets.0.insert(key.into(), value.into());
        self.write(&secrets, salt)
    }
    fn delete(&self, key: &str) -> io::Result<()> {
        let _guard = self.guard()?;
        let (mut secrets, salt) = self.read()?;
        let mut removed = secrets.0.remove(key).ok_or(io::ErrorKind::NotFound)?;
        removed.zeroize();
        self.write(&secrets, salt)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encrypted_store_persists_and_wrong_password_cannot_overwrite() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.json");
        let store = FileStore::new(path.clone(), "a long testing passphrase".into()).unwrap();
        store.put("record/access", "very-private-token").unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("very-private-token"));
        let reopened = FileStore::new(path.clone(), "a long testing passphrase".into()).unwrap();
        assert_eq!(reopened.get("record/access").unwrap(), "very-private-token");
        let wrong = FileStore::new(path.clone(), "a different testing passphrase".into()).unwrap();
        assert!(wrong.get("record/access").is_err());
        assert!(wrong.put("record/access", "replacement").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        reopened.delete("record/access").unwrap();
        assert_eq!(
            reopened.get("record/access").unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }
    #[test]
    fn tampering_is_detected_without_leaking_secrets() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("vault.json");
        let store = FileStore::new(path.clone(), "a long testing passphrase".into()).unwrap();
        store.put("key", "secret").unwrap();
        let mut envelope: Envelope =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        envelope.ciphertext[0] ^= 1;
        std::fs::write(&path, serde_json::to_vec(&envelope).unwrap()).unwrap();
        assert!(store.get("key").is_err());
        assert!(!store.get("key").unwrap_err().to_string().contains("secret"));
    }
}

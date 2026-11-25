//! Private identity material and an explicitly confirmed trust store.
use super::*;
use ed25519_dalek::{pkcs8::DecodePrivateKey, SigningKey};
use std::{collections::BTreeMap, fs::OpenOptions};
#[derive(Serialize, Deserialize)]
struct Material {
    key: Vec<u8>,
    certificate: Vec<u8>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Peer {
    pub id: String,
    pub public_key: Vec<u8>,
    pub certificate: Vec<u8>,
    pub address: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerStatus {
    pub peer: Peer,
    pub confirmed: bool,
    pub sas: Option<String>,
}
#[derive(Default, Serialize, Deserialize)]
struct Trust {
    peers: BTreeMap<String, Peer>,
    pending: BTreeMap<String, Pending>,
}
#[derive(Serialize, Deserialize)]
struct Pending {
    peer: Peer,
    sas: String,
    expires: u64,
}
pub(super) struct Identity {
    pub directory: PathBuf,
    pub key: SigningKey,
    pub key_der: Vec<u8>,
    pub certificate: Vec<u8>,
}
impl Identity {
    pub fn open(directory: &Path) -> io::Result<Self> {
        std::fs::create_dir_all(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
        }
        let _guard = guard(directory)?;
        let path = directory.join("identity.json");
        let material: Material = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(error)?,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let key = rcgen::KeyPair::generate_for(&rcgen::PKCS_ED25519).map_err(error)?;
                let params =
                    rcgen::CertificateParams::new(vec!["savesync.local".into()]).map_err(error)?;
                let certificate = params.self_signed(&key).map_err(error)?;
                let material = Material {
                    key: key.serialize_der(),
                    certificate: certificate.der().to_vec(),
                };
                atomic_write(&path, &serde_json::to_vec(&material).map_err(error)?)?;
                material
            }
            Err(e) => return Err(e),
        };
        let key = SigningKey::from_pkcs8_der(&material.key).map_err(error)?;
        Ok(Self {
            directory: directory.into(),
            key,
            key_der: material.key,
            certificate: material.certificate,
        })
    }
    pub fn public(&self) -> Vec<u8> {
        self.key.verifying_key().to_bytes().to_vec()
    }
    pub fn id(&self) -> String {
        id(&self.public())
    }
    pub fn peer(&self, address: Option<String>) -> Peer {
        Peer {
            id: self.id(),
            public_key: self.public(),
            certificate: self.certificate.clone(),
            address,
        }
    }
    fn read(&self) -> io::Result<Trust> {
        match std::fs::read(self.directory.join("trust.json")) {
            Ok(bytes) => serde_json::from_slice(&bytes).map_err(error),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Trust::default()),
            Err(e) => Err(e),
        }
    }
    fn write(&self, trust: &Trust) -> io::Result<()> {
        atomic_write(
            &self.directory.join("trust.json"),
            &serde_json::to_vec(trust).map_err(error)?,
        )
    }
    pub fn peers(&self) -> io::Result<Vec<PeerStatus>> {
        let _guard = guard(&self.directory)?;
        let trust = self.read()?;
        Ok(trust
            .peers
            .into_values()
            .map(|peer| PeerStatus {
                peer,
                confirmed: true,
                sas: None,
            })
            .chain(
                trust
                    .pending
                    .into_values()
                    .filter(|p| p.expires > crate::oauth::now_seconds())
                    .map(|p| PeerStatus {
                        peer: p.peer,
                        confirmed: false,
                        sas: Some(p.sas),
                    }),
            )
            .collect())
    }
    pub fn trusted(&self, public: &[u8]) -> io::Result<Peer> {
        let _guard = guard(&self.directory)?;
        self.read()?
            .peers
            .get(&id(public))
            .cloned()
            .ok_or_else(denied)
    }
    pub fn at_address(&self, address: &str) -> io::Result<Peer> {
        let _guard = guard(&self.directory)?;
        self.read()?
            .peers
            .into_values()
            .find(|p| p.address.as_deref() == Some(address))
            .ok_or_else(denied)
    }
    pub fn pending(&self, peer: Peer, sas: String) -> io::Result<()> {
        if peer.public_key.len() != 32
            || peer.certificate.len() > 16384
            || peer.id != id(&peer.public_key)
            || peer.id == self.id()
        {
            return Err(denied());
        }
        let _guard = guard(&self.directory)?;
        let mut trust = self.read()?;
        if let Some(existing) = trust.peers.get(&peer.id) {
            if existing.certificate != peer.certificate {
                return Err(denied());
            }
        }
        trust
            .pending
            .retain(|_, p| p.expires > crate::oauth::now_seconds());
        if trust.pending.len() >= 8 {
            return Err(error("too many pending pairings"));
        }
        trust.pending.insert(
            peer.id.clone(),
            Pending {
                peer,
                sas,
                expires: crate::oauth::now_seconds() + 300,
            },
        );
        self.write(&trust)
    }
    pub fn confirm(&self, peer_id: &str) -> io::Result<()> {
        let _guard = guard(&self.directory)?;
        let mut trust = self.read()?;
        let pending = trust
            .pending
            .remove(peer_id)
            .filter(|p| p.expires > crate::oauth::now_seconds())
            .ok_or_else(denied)?;
        let mut peer = pending.peer;
        if peer.address.is_none() {
            peer.address = trust
                .peers
                .get(peer_id)
                .and_then(|existing| existing.address.clone());
        }
        trust.peers.insert(peer_id.into(), peer);
        self.write(&trust)
    }
    pub fn forget(&self, peer_id: &str) -> io::Result<()> {
        let _guard = guard(&self.directory)?;
        let mut trust = self.read()?;
        trust.peers.remove(peer_id);
        trust.pending.remove(peer_id);
        self.write(&trust)
    }
}
pub(super) fn id(public: &[u8]) -> String {
    blake3::hash(public).to_hex().to_string()
}
fn guard(directory: &Path) -> io::Result<std::fs::File> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("identity.lock"))?;
    fs2::FileExt::lock_exclusive(&file)?;
    Ok(file)
}
fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    use std::io::Write;
    let temp = path.with_extension("tmp");
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temp)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    drop(file);
    std::fs::rename(temp, path)
}

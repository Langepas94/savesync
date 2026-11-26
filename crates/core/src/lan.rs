//! Authenticated LAN transport: QUIC/TLS 1.3, SPAKE2 enrollment and pinned identities.
use crate::backend::{Backend, FsBackend};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    time::Duration,
};
mod client;
mod discovery;
mod identity;
mod pairing;
mod server;
mod transport;
pub use client::LanBackend;
pub use discovery::{discover, DiscoveredPeer};
pub use identity::{Peer, PeerStatus};
pub use pairing::PairingOffer;
pub use server::LanServer;
const MAX_BLOB: usize = 512 * 1024 * 1024;
const MAX_HEADER: usize = 64 * 1024;
fn error(message: impl ToString) -> io::Error {
    io::Error::other(message.to_string())
}
fn denied() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "LAN trust/authentication failed; pair and confirm SAS on both devices",
    )
}
fn runtime() -> io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}
fn default_identity() -> PathBuf {
    crate::config::data_dir(crate::paths::Platform::current())
        .unwrap_or_else(|| PathBuf::from(".savesync"))
        .join("state/lan-identity")
}
pub fn peers(directory: &Path) -> io::Result<Vec<PeerStatus>> {
    identity::Identity::open(directory)?.peers()
}
pub fn confirm(directory: &Path, peer_id: &str) -> io::Result<()> {
    identity::Identity::open(directory)?.confirm(peer_id)
}
pub fn forget(directory: &Path, peer_id: &str) -> io::Result<()> {
    identity::Identity::open(directory)?.forget(peer_id)
}
pub fn reject(directory: &Path, peer_id: &str) -> io::Result<()> {
    identity::Identity::open(directory)?.reject(peer_id)
}
pub fn pair(directory: &Path, address: &str, pin: &str) -> io::Result<PairingOffer> {
    pair_named(directory, address, pin, &crate::config::hostname())
}
pub fn pair_named(
    directory: &Path,
    address: &str,
    pin: &str,
    device_name: &str,
) -> io::Result<PairingOffer> {
    runtime()?.block_on(pairing::enroll(directory, address, pin, device_name))
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Operation {
    Ping,
    GetIndex,
    PutIndex,
    PublishIndex { writer: String },
    GetSnapshot { game: String, version: String },
    PutSnapshot { game: String, version: String },
    ListSnapshots { game: String },
}
#[derive(Serialize, Deserialize)]
struct Request {
    operation: Operation,
    blob_len: usize,
}
#[derive(Serialize, Deserialize, Default)]
struct Response {
    error: Option<String>,
    not_found: bool,
    blob_len: usize,
    list: Vec<String>,
}
#[cfg(test)]
mod tests;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum LanAction {
    Status,
    Discover,
    Invite,
    ClosePairing,
    Pair { address: String, pin: String },
    Confirm { peer_id: String },
    Reject { peer_id: String },
    Forget { peer_id: String },
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LanPayload {
    pub identity: String,
    pub port: Option<u16>,
    pub peers: Vec<PeerStatus>,
    pub discovered: Vec<DiscoveredPeer>,
    pub invitation_pin: Option<String>,
    #[serde(default)]
    pub invitation_addresses: Vec<String>,
    pub offer: Option<PairingOffer>,
    pub error: Option<String>,
}

pub fn local_identity(directory: &Path) -> io::Result<String> {
    Ok(identity::Identity::open(directory)?.id())
}
pub fn invitation_pin() -> String {
    let entropy = oauth2::CsrfToken::new_random_len(32);
    let digest = blake3::hash(entropy.secret().as_bytes());
    format!(
        "{:06}",
        u32::from_le_bytes(digest.as_bytes()[..4].try_into().unwrap()) % 1_000_000
    )
}

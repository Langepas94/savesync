//! Synchronous Backend facade over a bounded, authenticated QUIC request.
use super::*;
use super::{identity::Identity, pairing::Hello, transport::*};
use ed25519_dalek::Signer;
pub struct LanBackend {
    address: String,
    directory: PathBuf,
}
impl LanBackend {
    /// Legacy PIN configuration no longer authorizes requests; explicit pairing is required.
    pub fn new(address: impl Into<String>, _legacy_pin: &str) -> Self {
        Self::with_identity(address, default_identity())
    }
    pub fn with_identity(address: impl Into<String>, directory: impl Into<PathBuf>) -> Self {
        Self {
            address: address.into(),
            directory: directory.into(),
        }
    }
    pub fn ping(&self) -> io::Result<()> {
        self.request(Operation::Ping, &[]).map(|_| ())
    }

    pub fn ping_peer(directory: &Path, address: &str, public_key: &[u8]) -> io::Result<()> {
        let identity = Identity::open(directory)?;
        let peer = identity.trusted(public_key)?;
        runtime()?.block_on(async {
            tokio::time::timeout(
                Duration::from_secs(2),
                Self::request_as(address, &identity, peer, Operation::Ping, &[]),
            )
            .await
            .map_err(error)?
            .map(|_| ())
        })
    }

    fn request(
        &self,
        operation: Operation,
        blob: &[u8],
    ) -> io::Result<(Response, Option<Vec<u8>>)> {
        let identity = Identity::open(&self.directory)?;
        let peer = identity.at_address(&self.address)?;
        runtime()?.block_on(Self::request_as(
            &self.address,
            &identity,
            peer,
            operation,
            blob,
        ))
    }

    async fn request_as(
        address: &str,
        identity: &Identity,
        peer: Peer,
        operation: Operation,
        blob: &[u8],
    ) -> io::Result<(Response, Option<Vec<u8>>)> {
        let (endpoint, connection) = connect(address, Some(peer.certificate)).await?;
        let result = tokio::time::timeout(Duration::from_secs(30), async {
            let (mut send, mut receive) = connection.open_bi().await.map_err(error)?;
            let signature = identity
                .key
                .sign(&super::server::authentication_message(&connection)?)
                .to_bytes()
                .to_vec();
            send_value(
                &mut send,
                &Hello::Authenticate {
                    public: identity.public(),
                    signature,
                },
            )
            .await?;
            let _: Response = receive_value(&mut receive).await?;
            send_value(
                &mut send,
                &Request {
                    operation,
                    blob_len: blob.len(),
                },
            )
            .await?;
            if !blob.is_empty() {
                send_bytes(&mut send, blob, MAX_BLOB).await?;
            }
            send.finish().map_err(error)?;
            let response: Response = receive_value(&mut receive).await?;
            if let Some(message) = &response.error {
                return Err(io::Error::new(
                    if response.not_found {
                        io::ErrorKind::NotFound
                    } else {
                        io::ErrorKind::Other
                    },
                    message.clone(),
                ));
            }
            let blob = if response.blob_len > 0 {
                let bytes = receive_bytes(&mut receive, MAX_BLOB).await?;
                if bytes.len() != response.blob_len {
                    return Err(error("response blob length mismatch"));
                }
                Some(bytes)
            } else {
                None
            };
            Ok((response, blob))
        })
        .await
        .map_err(error)?;
        connection.close(0u32.into(), b"request complete");
        endpoint.wait_idle().await;
        result
    }
}
impl Backend for LanBackend {
    fn name(&self) -> &'static str {
        "lan"
    }
    fn get_index(&self) -> io::Result<Option<Vec<u8>>> {
        self.request(Operation::GetIndex, &[]).map(|(_, blob)| blob)
    }
    fn publish_index(&self, data: &[u8], writer: &str) -> io::Result<()> {
        self.request(
            Operation::PublishIndex {
                writer: writer.into(),
            },
            data,
        )
        .map(|_| ())
    }
    fn put_index(&self, data: &[u8]) -> io::Result<()> {
        self.request(Operation::PutIndex, data).map(|_| ())
    }
    fn get_snapshot(&self, game: &str, version: &str) -> io::Result<Vec<u8>> {
        self.request(
            Operation::GetSnapshot {
                game: game.into(),
                version: version.into(),
            },
            &[],
        )?
        .1
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "empty snapshot"))
    }
    fn put_snapshot(&self, game: &str, version: &str, data: &[u8]) -> io::Result<()> {
        self.request(
            Operation::PutSnapshot {
                game: game.into(),
                version: version.into(),
            },
            data,
        )
        .map(|_| ())
    }
    fn list_snapshots(&self, game: &str) -> io::Result<Vec<String>> {
        self.request(Operation::ListSnapshots { game: game.into() }, &[])
            .map(|(response, _)| response.list)
    }
}
use super::transport::{receive as receive_value, send as send_value};

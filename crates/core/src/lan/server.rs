//! Bounded QUIC service: enrollment is separate from all storage operations.
use super::*;
use super::{
    identity::Identity,
    pairing::{Hello, PairingWindow},
    transport::*,
};
use ed25519_dalek::{Signature, VerifyingKey};
use std::sync::{Arc, Mutex};
pub struct LanServer {
    pub port: u16,
    endpoint: quinn::Endpoint,
    window: PairingWindow,
    task: Option<std::thread::JoinHandle<()>>,
    _discovery: Option<discovery::Advertisement>,
}
impl LanServer {
    pub fn start(store: PathBuf, port: u16, _legacy_pin: &str) -> io::Result<Self> {
        let directory = store
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("lan-identity");
        Self::start_with_identity(store, port, &directory, true)
    }
    pub fn start_with_identity(
        store: PathBuf,
        port: u16,
        directory: &Path,
        advertise: bool,
    ) -> io::Result<Self> {
        let identity = Arc::new(Identity::open(directory)?);
        let window = Arc::new(Mutex::new(None));
        let runtime = runtime()?;
        let endpoint = {
            let _entered = runtime.enter();
            quinn::Endpoint::server(
                server_config(&identity)?,
                format!("0.0.0.0:{port}").parse().map_err(error)?,
            )?
        };
        let port = endpoint.local_addr()?.port();
        let service = if advertise {
            discovery::Advertisement::start(&identity.id(), &crate::config::hostname(), port).ok()
        } else {
            None
        };
        let server_endpoint = endpoint.clone();
        let server_window = window.clone();
        let task = std::thread::spawn(move || {
            runtime.block_on(async move {
                let capacity = Arc::new(tokio::sync::Semaphore::new(8));
                while let Some(incoming) = server_endpoint.accept().await {
                    let Ok(permit) = capacity.clone().try_acquire_owned() else {
                        incoming.refuse();
                        continue;
                    };
                    let identity = identity.clone();
                    let store = store.clone();
                    let window = server_window.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        let _ = tokio::time::timeout(Duration::from_secs(30), async move {
                            let connection = incoming.await.map_err(error)?;
                            let result = handle(&connection, &identity, &store, &window).await;
                            if result.is_err() {
                                connection.close(1u32.into(), b"authentication or request failed");
                            }
                            result
                        })
                        .await;
                    });
                }
            })
        });
        Ok(Self {
            port,
            endpoint,
            window,
            task: Some(task),
            _discovery: service,
        })
    }
    pub fn open_pairing(&self, pin: &str) -> io::Result<()> {
        pairing::open(&self.window, pin)
    }
    pub fn close_pairing(&self) {
        if let Ok(mut window) = self.window.lock() {
            *window = None;
        }
    }
    /// A config change closes enrollment; established device trust uses pinned keys.
    pub fn set_pin(&self, _pin: &str) {
        self.close_pairing();
    }
}
impl Drop for LanServer {
    fn drop(&mut self) {
        self.endpoint.close(0u32.into(), b"server stopped");
        if let Some(task) = self.task.take() {
            let _ = task.join();
        }
    }
}
async fn handle(
    connection: &quinn::Connection,
    identity: &Identity,
    store: &Path,
    window: &PairingWindow,
) -> io::Result<()> {
    let (mut send, mut receive) = connection.accept_bi().await.map_err(error)?;
    let hello: Hello = tokio::time::timeout(Duration::from_secs(5), receive_value(&mut receive))
        .await
        .map_err(error)??;
    match hello {
        Hello::Pair { peer, spake } => {
            pairing::accept(
                identity,
                connection,
                &mut send,
                &mut receive,
                peer,
                spake,
                window,
            )
            .await?;
            send.finish().map_err(error)?;
            let _ = send.stopped().await;
            return Ok(());
        }
        Hello::Authenticate { public, signature } => {
            let _peer = identity.trusted(&public)?;
            let public: [u8; 32] = public.try_into().map_err(|_| denied())?;
            let key = VerifyingKey::from_bytes(&public).map_err(|_| denied())?;
            let signature = Signature::from_slice(&signature).map_err(|_| denied())?;
            key.verify_strict(&authentication_message(connection)?, &signature)
                .map_err(|_| denied())?;
            send_value(&mut send, &Response::default()).await?;
        }
    }
    let request: Request = receive_value(&mut receive).await?;
    if request.blob_len > MAX_BLOB {
        return Err(error("snapshot too large"));
    }
    let blob = if request.blob_len > 0 {
        let bytes = receive_bytes(&mut receive, MAX_BLOB).await?;
        if bytes.len() != request.blob_len {
            return Err(error("blob length mismatch"));
        }
        bytes
    } else {
        vec![]
    };
    let backend = FsBackend::new(store);
    let mut response = Response::default();
    let outcome: io::Result<Option<Vec<u8>>> = match request.operation {
        Operation::Ping => Ok(None),
        Operation::GetIndex => backend.get_index(),
        Operation::PutIndex => backend.put_index(&blob).map(|_| None),
        Operation::GetSnapshot { game, version } => backend.get_snapshot(&game, &version).map(Some),
        Operation::PutSnapshot { game, version } => {
            backend.put_snapshot(&game, &version, &blob).map(|_| None)
        }
        Operation::ListSnapshots { game } => backend.list_snapshots(&game).map(|list| {
            response.list = list;
            None
        }),
    };
    let output = match outcome {
        Ok(blob) => blob,
        Err(e) => {
            response.not_found = e.kind() == io::ErrorKind::NotFound;
            response.error = Some(e.to_string());
            None
        }
    };
    response.blob_len = output.as_ref().map_or(0, Vec::len);
    send_value(&mut send, &response).await?;
    if let Some(bytes) = output {
        send_bytes(&mut send, &bytes, MAX_BLOB).await?;
    }
    send.finish().map_err(error)?;
    let _ = send.stopped().await;
    Ok(())
}
pub(super) fn authentication_message(connection: &quinn::Connection) -> io::Result<Vec<u8>> {
    let mut bytes = b"SaveSync authenticated client v2".to_vec();
    bytes.extend(binding(connection)?);
    Ok(bytes)
}
use super::transport::{receive as receive_value, send as send_value};

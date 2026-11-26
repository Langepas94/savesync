//! SPAKE2 proofs bind both identities and the TLS exporter; a relay cannot enroll itself.
use super::*;
use super::{
    identity::{Identity, Peer},
    transport::*,
};
use spake2::{Ed25519Group, Identity as SpakeIdentity, Password, Spake2};
use std::sync::{Arc, Mutex};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PairingOffer {
    pub peer_id: String,
    pub sas: String,
}
pub(super) struct Window {
    generation: String,
    pin: String,
    until: std::time::Instant,
    attempts: u8,
}
pub(super) type PairingWindow = Arc<Mutex<Option<Window>>>;
pub(super) fn open(window: &PairingWindow, pin: &str) -> io::Result<()> {
    if pin.len() != 6 || !pin.bytes().all(|b| b.is_ascii_digit()) {
        return Err(error("pairing PIN must contain six digits"));
    }
    *window.lock().map_err(error)? = Some(Window {
        generation: oauth2::CsrfToken::new_random().secret().clone(),
        pin: pin.into(),
        until: std::time::Instant::now() + Duration::from_secs(120),
        attempts: 0,
    });
    Ok(())
}
pub(super) fn take_attempt(window: &PairingWindow) -> io::Result<(String, String)> {
    let mut window = window.lock().map_err(error)?;
    let window = window.as_mut().ok_or_else(denied)?;
    if window.until < std::time::Instant::now() || window.attempts >= 5 {
        return Err(denied());
    }
    window.attempts += 1;
    Ok((window.pin.clone(), window.generation.clone()))
}
pub(super) fn active_pin(window: &PairingWindow) -> Option<String> {
    window
        .lock()
        .ok()?
        .as_ref()
        .filter(|window| window.until > std::time::Instant::now() && window.attempts < 5)
        .map(|window| window.pin.clone())
}
#[derive(Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(super) enum Hello {
    Pair { peer: Peer, spake: Vec<u8> },
    Authenticate { public: Vec<u8>, signature: Vec<u8> },
}
#[derive(Serialize, Deserialize)]
struct Answer {
    peer: Peer,
    spake: Vec<u8>,
    proof: Vec<u8>,
    #[serde(default)]
    name: Option<ProvenName>,
}
#[derive(Serialize, Deserialize)]
struct Proof {
    proof: Vec<u8>,
    #[serde(default)]
    name: Option<ProvenName>,
}
// Name proofs extend the handshake without changing the transcript used by older peers.
#[derive(Serialize, Deserialize)]
struct ProvenName {
    value: String,
    proof: Vec<u8>,
}
impl ProvenName {
    fn new(key: &[u8], transcript: &Transcript<'_>, value: &str, role: &str) -> io::Result<Self> {
        let value = if value.trim().is_empty() {
            "SaveSync"
        } else {
            value.trim()
        };
        let value: String = value.chars().take(128).collect();
        let proof = digest(key, transcript, &format!("{role} name:{value}"))?;
        Ok(Self { value, proof })
    }

    fn verify(self, key: &[u8], transcript: &Transcript<'_>, role: &str) -> io::Result<String> {
        if self.value.chars().count() > 128 || self.value.trim().is_empty() {
            return Err(denied());
        }
        verify(
            &self.proof,
            &digest(key, transcript, &format!("{role} name:{}", self.value))?,
        )?;
        Ok(self.value)
    }
}
#[derive(Serialize)]
struct Transcript<'a> {
    binding: [u8; 32],
    client: &'a Peer,
    server: &'a Peer,
    a: &'a [u8],
    b: &'a [u8],
}
fn digest(key: &[u8], transcript: &Transcript<'_>, role: &str) -> io::Result<Vec<u8>> {
    let key = blake3::derive_key("SaveSync SPAKE2 confirmation v2", key);
    let mut hash = blake3::Hasher::new_keyed(&key);
    hash.update(role.as_bytes());
    hash.update(&serde_json::to_vec(transcript).map_err(error)?);
    Ok(hash.finalize().as_bytes().to_vec())
}
fn verify(actual: &[u8], expected: &[u8]) -> io::Result<()> {
    use subtle::ConstantTimeEq;
    if actual.ct_eq(expected).unwrap_u8() != 1 {
        return Err(denied());
    }
    Ok(())
}
fn sas(key: &[u8], transcript: &Transcript<'_>) -> io::Result<String> {
    let hash = digest(key, transcript, "SAS")?;
    let number = u32::from_le_bytes(hash[..4].try_into().unwrap()) % 1_000_000;
    Ok(format!("{number:06}"))
}
pub(super) async fn accept(
    identity: &Identity,
    connection: &quinn::Connection,
    streams: (&mut quinn::SendStream, &mut quinn::RecvStream),
    client: Peer,
    a: Vec<u8>,
    window: &PairingWindow,
    device_name: &str,
) -> io::Result<()> {
    let (send, receive) = streams;
    let (pin, generation) = take_attempt(window)?;
    if a.len() > 128 || client.public_key.len() != 32 || client.certificate.len() > 16384 {
        return Err(denied());
    }
    let (state, b) = Spake2::<Ed25519Group>::start_b(
        &Password::new(pin.as_bytes()),
        &SpakeIdentity::new(b"SaveSync client v2"),
        &SpakeIdentity::new(b"SaveSync server v2"),
    );
    let key = state.finish(&a).map_err(error)?;
    let server = identity.peer(None);
    let transcript = Transcript {
        binding: binding(connection)?,
        client: &client,
        server: &server,
        a: &a,
        b: &b,
    };
    send_value(
        send,
        &Answer {
            peer: server.clone(),
            spake: b.clone(),
            proof: digest(&key, &transcript, "server")?,
            name: Some(ProvenName::new(&key, &transcript, device_name, "server")?),
        },
    )
    .await?;
    let proof: Proof = receive_value(receive).await?;
    verify(&proof.proof, &digest(&key, &transcript, "client")?)?;
    let sas = sas(&key, &transcript)?;
    let name = proof
        .name
        .map(|name| name.verify(&key, &transcript, "client"))
        .transpose()?;
    let mut client = client;
    client.name = name;
    let offer = PairingOffer {
        peer_id: server.id.clone(),
        sas: sas.clone(),
    };
    {
        let mut current = window.lock().map_err(error)?;
        if !current.as_ref().is_some_and(|window| {
            window.generation == generation && window.until > std::time::Instant::now()
        }) {
            return Err(denied());
        }
        identity.pending(client, sas)?;
        // Consume only this invitation; cancellation invalidates in-flight PAKE too.
        *current = None;
    }
    send_value(send, &offer).await
}
// Aliases keep the stream variable names readable in the handshake.
use super::transport::{receive as receive_value, send as send_value};
pub(super) async fn enroll(
    directory: &Path,
    address: &str,
    pin: &str,
    device_name: &str,
) -> io::Result<PairingOffer> {
    if pin.len() != 6 || !pin.bytes().all(|b| b.is_ascii_digit()) {
        return Err(denied());
    }
    let identity = Identity::open(directory)?;
    let (endpoint, connection) = connect(address, None).await?;
    let outcome = tokio::time::timeout(Duration::from_secs(10), async {
        let (mut send, mut receive) = connection.open_bi().await.map_err(error)?;
        let (state, a) = Spake2::<Ed25519Group>::start_a(
            &Password::new(pin.as_bytes()),
            &SpakeIdentity::new(b"SaveSync client v2"),
            &SpakeIdentity::new(b"SaveSync server v2"),
        );
        let client = identity.peer(None);
        send_value(
            &mut send,
            &Hello::Pair {
                peer: client.clone(),
                spake: a.clone(),
            },
        )
        .await?;
        let answer: Answer = receive_value(&mut receive).await?;
        if answer.peer.certificate != certificate(&connection)? || answer.spake.len() > 128 {
            return Err(denied());
        }
        let key = state.finish(&answer.spake).map_err(error)?;
        let transcript = Transcript {
            binding: binding(&connection)?,
            client: &client,
            server: &answer.peer,
            a: &a,
            b: &answer.spake,
        };
        verify(&answer.proof, &digest(&key, &transcript, "server")?)?;
        send_value(
            &mut send,
            &Proof {
                proof: digest(&key, &transcript, "client")?,
                name: Some(ProvenName::new(&key, &transcript, device_name, "client")?),
            },
        )
        .await?;
        let offer: PairingOffer = receive_value(&mut receive).await?;
        let expected = sas(&key, &transcript)?;
        let name = answer
            .name
            .map(|name| name.verify(&key, &transcript, "server"))
            .transpose()?;
        if offer.peer_id != answer.peer.id || offer.sas != expected {
            return Err(denied());
        }
        let mut peer = answer.peer;
        peer.address = Some(address.into());
        peer.name = name;
        identity.pending(peer, expected)?;
        send.finish().map_err(error)?;
        Ok(offer)
    })
    .await
    .map_err(error)?;
    connection.close(0u32.into(), b"enrollment complete");
    endpoint.wait_idle().await;
    outcome
}

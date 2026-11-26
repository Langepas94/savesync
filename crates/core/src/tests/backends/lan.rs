use super::*;
use tempfile::TempDir;
struct Pair {
    _root: TempDir,
    server: LanServer,
    client: PathBuf,
    server_identity: PathBuf,
    address: String,
}
impl Pair {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let client = root.path().join("client");
        let server_identity = root.path().join("server");
        let server =
            LanServer::start_with_identity(root.path().join("store"), 0, &server_identity, false)
                .unwrap();
        let address = format!("127.0.0.1:{}", server.port);
        Self {
            _root: root,
            server,
            client,
            server_identity,
            address,
        }
    }
    fn enroll(&self) {
        self.server.open_pairing("123456").unwrap();
        let offer = pair(&self.client, &self.address, "123456").unwrap();
        let pending = peers(&self.server_identity).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].sas.as_deref(), Some(offer.sas.as_str()));
        confirm(&self.client, &offer.peer_id).unwrap();
        confirm(&self.server_identity, &pending[0].peer.id).unwrap();
    }
    fn backend(&self) -> LanBackend {
        LanBackend::with_identity(&self.address, &self.client)
    }
}
#[test]
fn pairing_requires_invitation_matching_pin_and_both_sas_confirmations() {
    let pair = Pair::new();
    assert!(pair.backend().ping().is_err());
    assert!(super::pair(&pair.client, &pair.address, "123456").is_err());
    pair.server.open_pairing("123456").unwrap();
    assert!(super::pair(&pair.client, &pair.address, "654321").is_err());
    let offer = super::pair(&pair.client, &pair.address, "123456").unwrap();
    assert!(pair.backend().ping().is_err());
    confirm(&pair.client, &offer.peer_id).unwrap();
    assert!(pair.backend().ping().is_err());
    let remote = peers(&pair.server_identity).unwrap().pop().unwrap();
    confirm(&pair.server_identity, &remote.peer.id).unwrap();
    pair.backend().ping().unwrap();
    // The invitation is consumed and the PIN is not an ongoing bearer secret.
    assert!(super::pair(&pair._root.path().join("outsider"), &pair.address, "123456").is_err());
    pair.server.set_pin("changed");
    pair.backend().ping().unwrap();
    forget(&pair.server_identity, &remote.peer.id).unwrap();
    assert!(pair.backend().ping().is_err());
}
#[test]
fn authenticated_snapshot_roundtrip_and_untrusted_write_rejected() {
    let pair = Pair::new();
    pair.enroll();
    let backend = pair.backend();
    assert!(backend.get_index().unwrap().is_none());
    backend.put_index(br#"{"version":1}"#).unwrap();
    assert_eq!(backend.get_index().unwrap().unwrap(), br#"{"version":1}"#);
    backend
        .put_snapshot("test-game", "v1_pc_abcdef12", b"snapshot")
        .unwrap();
    assert_eq!(
        backend.get_snapshot("test-game", "v1_pc_abcdef12").unwrap(),
        b"snapshot"
    );
    assert_eq!(
        backend.list_snapshots("test-game").unwrap(),
        vec!["v1_pc_abcdef12"]
    );
    assert_eq!(
        backend
            .get_snapshot("test-game", "v2_pc_abcdef12")
            .unwrap_err()
            .kind(),
        io::ErrorKind::NotFound
    );
    let outsider = LanBackend::with_identity(&pair.address, pair._root.path().join("outsider"));
    assert!(outsider.put_index(b"overwrite").is_err());
    assert_eq!(backend.get_index().unwrap().unwrap(), br#"{"version":1}"#);
    assert!(backend
        .put_snapshot("../bad", "v1_pc_abcdef12", b"bad")
        .is_err());
}
#[test]
fn restarted_identity_stays_pinned_and_changed_server_is_rejected() {
    let mut pair = Pair::new();
    pair.enroll();
    let old_port = pair.server.port;
    pair.server = LanServer::start_with_identity(
        pair._root.path().join("other-store"),
        0,
        &pair.server_identity,
        false,
    )
    .unwrap();
    // Restart on the original endpoint using the same on-disk key.
    pair.server = LanServer::start_with_identity(
        pair._root.path().join("store"),
        old_port,
        &pair.server_identity,
        false,
    )
    .unwrap();
    pair.backend().ping().unwrap();
    pair.server = LanServer::start_with_identity(
        pair._root.path().join("elsewhere"),
        0,
        &pair._root.path().join("replacement"),
        false,
    )
    .unwrap();
    pair.server = LanServer::start_with_identity(
        pair._root.path().join("elsewhere"),
        old_port,
        &pair._root.path().join("replacement"),
        false,
    )
    .unwrap();
    assert!(pair.backend().ping().is_err());
}
#[test]
fn engine_over_secure_lan_two_devices() {
    let pair = Pair::new();
    pair.enroll();
    let home = pair._root.path().join("home");
    std::fs::create_dir_all(&home).unwrap();
    let save = home.join("slot.sav");
    std::fs::write(&save, "progress").unwrap();
    let location = crate::catalog::scanner::SaveLocation {
        location: None,
        pattern: "<home>/*.sav".into(),
        resolved: home.to_string_lossy().into_owned(),
        hits: vec![save],
    };
    let pc =
        crate::sync::engine::Engine::new(pair.backend(), "pc", pair._root.path().join("pc-state"));
    assert!(matches!(
        pc.push("test-game", &[location]).unwrap(),
        crate::sync::engine::PushOutcome::Uploaded(_)
    ));
    let deck = crate::sync::engine::Engine::new(
        pair.backend(),
        "deck",
        pair._root.path().join("deck-state"),
    );
    assert!(deck.has_remote_versions("test-game").unwrap());
}

#[test]
fn authentication_signature_cannot_be_replayed_on_another_tls_session() {
    use ed25519_dalek::Signer;
    let pair = Pair::new();
    pair.enroll();
    let identity = identity::Identity::open(&pair.client).unwrap();
    let peer = identity.at_address(&pair.address).unwrap();
    runtime().unwrap().block_on(async {
        let (endpoint, first) = transport::connect(&pair.address, Some(peer.certificate.clone()))
            .await
            .unwrap();
        let signature = identity
            .key
            .sign(&server::authentication_message(&first).unwrap())
            .to_bytes()
            .to_vec();
        first.close(0u32.into(), b"first session closed");
        endpoint.wait_idle().await;
        let (endpoint, second) = transport::connect(&pair.address, Some(peer.certificate))
            .await
            .unwrap();
        let (mut send, mut receive) = second.open_bi().await.unwrap();
        transport::send(
            &mut send,
            &pairing::Hello::Authenticate {
                public: identity.public(),
                signature,
            },
        )
        .await
        .unwrap();
        assert!(transport::receive::<Response>(&mut receive).await.is_err());
        second.close(0u32.into(), b"done");
        endpoint.wait_idle().await;
    });
}
#[test]
fn pairing_window_limits_online_guesses_and_pending_trust_expires() {
    let window = std::sync::Arc::new(std::sync::Mutex::new(None));
    pairing::open(&window, "123456").unwrap();
    for _ in 0..5 {
        pairing::take_attempt(&window).unwrap();
    }
    assert!(pairing::take_attempt(&window).is_err());
    let pair = Pair::new();
    pair.server.open_pairing("123456").unwrap();
    let offer = super::pair(&pair.client, &pair.address, "123456").unwrap();
    let path = pair.client.join("trust.json");
    let mut trust: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    trust["pending"][&offer.peer_id]["expires"] = 0.into();
    std::fs::write(&path, serde_json::to_vec(&trust).unwrap()).unwrap();
    assert!(confirm(&pair.client, &offer.peer_id).is_err());
    assert!(peers(&pair.client).unwrap().is_empty());
}

#[test]
fn reverse_repair_preserves_outgoing_address() {
    let pair = Pair::new();
    pair.enroll();
    let identity = identity::Identity::open(&pair.client).unwrap();
    let existing = identity.at_address(&pair.address).unwrap();
    let mut reverse = existing.clone();
    reverse.address = None;
    identity.pending(reverse, "123456".into()).unwrap();
    identity.confirm(&existing.id).unwrap();
    assert_eq!(identity.at_address(&pair.address).unwrap().id, existing.id);
    pair.backend().ping().unwrap();
}

use super::Runtime;
use savesync_core::backends::lan::LanBackend;
use savesync_core::backends::lan::{self, LanAction, LanPayload};
use std::io;
use std::path::PathBuf;
impl Runtime {
    pub fn lan_presence_directory(&self) -> Option<PathBuf> {
        self.config
            .backends
            .lan
            .as_ref()
            .map(|_| self.state_dir.join("lan-identity"))
    }

    pub fn lan_action(&mut self, action: LanAction) -> LanPayload {
        let directory = self.state_dir.join("lan-identity");
        let mut payload = LanPayload::default();
        let result = (|| -> io::Result<()> {
            match action {
                LanAction::Status => (),
                LanAction::Discover => {
                    let identity = lan::local_identity(&directory)?;
                    payload.discovered = lan::discover(std::time::Duration::from_secs(5))?
                        .into_iter()
                        .filter(|peer| peer.identity != identity)
                        .collect();
                }
                LanAction::Invite => {
                    let server = self
                        .lan_server
                        .as_ref()
                        .ok_or_else(|| io::Error::other("включите LAN и запустите демон"))?;
                    let entropy = lan::invitation_pin();
                    server.open_pairing(&entropy)?;
                }
                LanAction::ClosePairing => {
                    if let Some(server) = &self.lan_server {
                        server.close_pairing();
                    }
                }
                LanAction::Pair { address, pin } => {
                    payload.offer = Some(lan::pair_named(
                        &directory,
                        &address,
                        &pin,
                        &self.config.device_name,
                    )?)
                }
                LanAction::Confirm { peer_id } => {
                    let peers = lan::peers(&directory)?;
                    let peer = peers
                        .iter()
                        .find(|p| !p.confirmed && p.peer.id == peer_id)
                        .ok_or_else(|| io::Error::other("сопряжение не найдено"))?;
                    lan::confirm(&directory, &peer_id)?;
                    if let Some(address) = &peer.peer.address {
                        let previous = peers
                            .iter()
                            .find(|entry| entry.confirmed && entry.peer.id == peer_id)
                            .and_then(|entry| entry.peer.address.as_ref());
                        let mut config = self.config.clone();
                        // CLI clients can confirm an outgoing pairing before starting LAN.
                        let setting = config.backends.lan.get_or_insert_with(|| {
                            savesync_core::config::LanConfig {
                                port: 0,
                                pin: String::new(),
                                peers: Vec::new(),
                            }
                        });
                        if let Some(previous) = previous.filter(|previous| *previous != address) {
                            setting.peers.retain(|peer| peer != previous);
                        }
                        if !setting.peers.contains(address) {
                            setting.peers.push(address.clone());
                        }
                        if config.backends.lan != self.config.backends.lan
                            && self.save_config(config).is_err()
                        {
                            return Err(io::Error::other(
                                "Устройство подтверждено, но не удалось сохранить LAN-адрес. Проверьте настройки локальной сети.",
                            ));
                        }
                    }
                }
                LanAction::Forget { peer_id } => {
                    let address = lan::peers(&directory)?
                        .into_iter()
                        .find(|entry| entry.confirmed && entry.peer.id == peer_id)
                        .and_then(|entry| entry.peer.address);
                    if let Some(address) = address {
                        let mut config = self.config.clone();
                        if let Some(setting) = config.backends.lan.as_mut() {
                            if setting.peers.contains(&address) {
                                setting.peers.retain(|peer| peer != &address);
                                if self.save_config(config).is_err() {
                                    return Err(io::Error::other(
                                        "не удалось удалить LAN-адрес из настроек",
                                    ));
                                }
                            }
                        }
                    }
                    lan::forget(&directory, &peer_id)?;
                }
                LanAction::Reject { peer_id } => lan::reject(&directory, &peer_id)?,
            }
            payload.peers = lan::peers(&directory)?;
            payload.identity = lan::local_identity(&directory)?;
            payload.port = self.lan_server.as_ref().map(|server| server.port);
            payload.invitation_pin = self
                .lan_server
                .as_ref()
                .and_then(|server| server.active_pin());
            if let (Some(port), Some(_)) = (payload.port, &payload.invitation_pin) {
                payload.invitation_addresses = if_addrs::get_if_addrs()
                    .unwrap_or_default()
                    .into_iter()
                    .filter(|interface| !interface.is_loopback())
                    .filter_map(|interface| match interface.ip() {
                        std::net::IpAddr::V4(ip) if !ip.is_unspecified() && !ip.is_link_local() => {
                            Some(format!("{ip}:{port}"))
                        }
                        _ => None,
                    })
                    .collect();
                payload.invitation_addresses.sort_unstable();
                payload.invitation_addresses.dedup();
            }
            Ok(())
        })();
        if let Err(error) = result {
            payload.error = Some(error.to_string());
        }
        payload
    }
}

pub fn lan_presence(directory: Option<PathBuf>) -> io::Result<Vec<String>> {
    let Some(directory) = directory else {
        return Ok(Vec::new());
    };
    let peers: Vec<_> = lan::peers(&directory)?
        .into_iter()
        .filter(|peer| peer.confirmed)
        .collect();
    if peers.is_empty() {
        return Ok(Vec::new());
    }
    let discovered = lan::discover(std::time::Duration::from_millis(850)).unwrap_or_default();
    let mut online = Vec::new();
    for group in peers.chunks(8) {
        online.extend(std::thread::scope(|scope| {
            let probes: Vec<_> = group
                .iter()
                .map(|entry| {
                    let peer = &entry.peer;
                    let directory = &directory;
                    let discovered = &discovered;
                    scope.spawn(move || {
                        let addresses: std::collections::BTreeSet<_> = peer
                            .address
                            .iter()
                            .chain(
                                discovered
                                    .iter()
                                    .filter(|found| found.identity == peer.id)
                                    .map(|found| &found.address),
                            )
                            .take(4)
                            .collect();
                        addresses
                            .into_iter()
                            .any(|address| {
                                LanBackend::ping_peer(directory, address, &peer.public_key).is_ok()
                            })
                            .then(|| peer.id.clone())
                    })
                })
                .collect();
            probes
                .into_iter()
                .filter_map(|probe| probe.join().ok().flatten())
                .collect::<Vec<_>>()
        }));
    }
    Ok(online)
}

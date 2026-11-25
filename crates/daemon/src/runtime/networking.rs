use super::*;
use savesync_core::lan::{self, LanAction, LanPayload};
impl Runtime {
    pub fn lan_action(&mut self, action: LanAction) -> ResponsePayload {
        let directory = self.state_dir.join("lan-identity");
        let mut payload = LanPayload::default();
        let result =
            (|| -> io::Result<()> {
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
                        payload.invitation_pin = Some(entropy);
                    }
                    LanAction::ClosePairing => {
                        if let Some(server) = &self.lan_server {
                            server.close_pairing();
                        }
                    }
                    LanAction::Pair { address, pin } => {
                        payload.offer = Some(lan::pair(&directory, &address, &pin)?)
                    }
                    LanAction::Confirm { peer_id } => {
                        let peer = lan::peers(&directory)?
                            .into_iter()
                            .find(|p| p.peer.id == peer_id)
                            .ok_or_else(|| io::Error::other("сопряжение не найдено"))?;
                        lan::confirm(&directory, &peer_id)?;
                        if let Some(address) = peer.peer.address {
                            let mut config = self.config.clone();
                            let setting = config.backends.lan.get_or_insert(
                                savesync_core::config::LanConfig {
                                    port: 47660,
                                    pin: String::new(),
                                    peers: vec![],
                                },
                            );
                            if !setting.peers.contains(&address) {
                                setting.peers.push(address);
                            }
                            if !matches!(self.save_config(config), ResponsePayload::Config(_)) {
                                return Err(io::Error::other(
                                    "доверие сохранено, но адрес не сохранён; обновите настройки",
                                ));
                            }
                        }
                    }
                    LanAction::Forget { peer_id } => {
                        lan::forget(&directory, &peer_id)?;
                    }
                }
                payload.peers = lan::peers(&directory)?;
                payload.identity = lan::local_identity(&directory)?;
                payload.port = self.lan_server.as_ref().map(|server| server.port);
                Ok(())
            })();
        if let Err(error) = result {
            payload.error = Some(error.to_string());
        }
        ResponsePayload::Lan(payload)
    }
}

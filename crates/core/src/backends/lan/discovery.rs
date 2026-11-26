//! Discovery is advisory: no discovered device becomes trusted automatically.
use super::*;
use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
const SERVICE: &str = "_savesync._udp.local.";
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiscoveredPeer {
    pub identity: String,
    pub name: String,
    pub address: String,
}
pub(super) struct Advertisement {
    daemon: ServiceDaemon,
    name: String,
}
impl Advertisement {
    pub fn start(identity: &str, name: &str, port: u16) -> io::Result<Self> {
        let daemon = ServiceDaemon::new().map_err(error)?;
        let host = format!("ss-{}.local.", &identity[..16]);
        let instance = format!("SaveSync-{}", &identity[..16]);
        let info = ServiceInfo::new(
            SERVICE,
            &instance,
            &host,
            "",
            port,
            &[("identity", identity), ("name", name), ("protocol", "2")][..],
        )
        .map_err(error)?
        .enable_addr_auto();
        let name = info.get_fullname().to_string();
        daemon.register(info).map_err(error)?;
        Ok(Self { daemon, name })
    }
}
impl Drop for Advertisement {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.name);
        let _ = self.daemon.shutdown();
    }
}
pub fn discover(timeout: Duration) -> io::Result<Vec<DiscoveredPeer>> {
    let daemon = ServiceDaemon::new().map_err(error)?;
    let events = daemon.browse(SERVICE).map_err(error)?;
    let deadline = std::time::Instant::now() + timeout;
    let mut peers = Vec::new();
    while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
        match events.recv_timeout(remaining) {
            Ok(ServiceEvent::ServiceResolved(service)) => {
                if service.get_property_val_str("protocol") != Some("2") {
                    continue;
                }
                let identity = service.get_property_val_str("identity").unwrap_or_default();
                if identity.len() != 64 || !identity.bytes().all(|b| b.is_ascii_hexdigit()) {
                    continue;
                }
                for address in service.get_addresses_v4() {
                    let peer = DiscoveredPeer {
                        identity: identity.into(),
                        name: service
                            .get_property_val_str("name")
                            .unwrap_or("SaveSync")
                            .chars()
                            .take(128)
                            .collect(),
                        address: format!("{address}:{}", service.get_port()),
                    };
                    if !peers.contains(&peer) {
                        peers.push(peer);
                    }
                }
            }
            Ok(_) => (),
            Err(_) => break,
        }
    }
    let _ = daemon.stop_browse(SERVICE);
    let _ = daemon.shutdown();
    Ok(peers)
}

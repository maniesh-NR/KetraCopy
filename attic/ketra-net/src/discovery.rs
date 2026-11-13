//! mDNS discovery for `_localsend._tcp` on the LAN. Announces our identity
//! and browses for peers. The TXT record mirrors the LocalSend v2 schema:
//!
//! ```text
//! alias=Alice
//! version=2
//! deviceModel=Pixel 8
//! deviceType=mobile
//! fingerprint=<hex>
//! port=53317
//! download=true
//! ketra=1                  # 1 if accelerated extension is supported
//! ```
//!
//! The peer's IP is learned from the mDNS A/AAAA record; the rest of the
//! `DeviceInfo` is fetched lazily via `GET /info` when the user picks a peer.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::identity::{DeviceFingerprint, DeviceIdentity, DeviceType};
use crate::protocol::{DeviceInfo, KetraExtension};

/// A discovered peer.
#[derive(Debug, Clone)]
pub struct Peer {
    /// mDNS service name (typically the peer's hostname).
    pub service_name: String,
    /// Human alias.
    pub alias: String,
    /// Device type (from txt).
    pub device_type: DeviceType,
    /// Hex SHA-256 fingerprint (from txt; confirmed via TLS pinning on connect).
    pub fingerprint: String,
    /// HTTPS port.
    pub port: u16,
    /// Best IP address discovered.
    pub ip: Option<IpAddr>,
    /// True when the peer advertises the Ketra accelerated extension.
    pub ketra: bool,
}

/// Live peer registry. Updated by a background mDNS browse thread.
#[derive(Clone)]
pub struct PeerRegistry {
    inner: Arc<Mutex<HashMap<String, Peer>>>,
}

impl PeerRegistry {
    pub fn new() -> Self {
        Self { inner: Arc::new(Mutex::new(HashMap::new())) }
    }

    /// Snapshot of currently-known peers.
    pub fn snapshot(&self) -> Vec<Peer> {
        self.inner.lock().unwrap().values().cloned().collect()
    }

    fn upsert(&self, peer: Peer) {
        self.inner.lock().unwrap().insert(peer.service_name.clone(), peer);
    }

    fn remove(&self, service_name: &str) {
        self.inner.lock().unwrap().remove(service_name);
    }
}

impl Default for PeerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// mDNS event delivered to the caller's callback.
#[derive(Debug, Clone)]
pub enum PeerEvent {
    /// A new peer appeared, or an existing peer's info changed.
    Found(Peer),
    /// A peer left the LAN.
    Removed(String),
}

/// Start mDNS announce + browse. Returns the daemon handle (keep alive to
/// keep announcing) and a registry snapshot. Events are also delivered to
/// the optional callback.
pub fn start(
    id: &DeviceIdentity,
    ketra: &KetraExtension,
    on_event: Option<Box<dyn Fn(PeerEvent) + Send + Sync>>,
) -> std::io::Result<(ServiceDaemon, PeerRegistry)> {
    let daemon = ServiceDaemon::new()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("mdns: {e}")))?;

    // --- Announce our own service ---
    let host_name = format!("ketra-{}", &id.fingerprint.hex()[..8]);
    let mut txt: HashMap<String, String> = HashMap::new();
    txt.insert("alias".into(), id.alias.clone());
    txt.insert("version".into(), id.version.to_string());
    txt.insert("deviceModel".into(), id.model.clone());
    txt.insert("deviceType".into(), id.device_type.as_str().into());
    txt.insert("fingerprint".into(), id.fingerprint.hex());
    txt.insert("port".into(), id.port.to_string());
    txt.insert("download".into(), "true".into());
    txt.insert("ketra".into(), if ketra.accelerated { "1".into() } else { "0".into() });

    let service_info = ServiceInfo::new(
        crate::LOCALSEND_SERVICE,
        &host_name,
        &host_name,
        "",
        id.port,
        txt,
    )
    .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("mdns info: {e}")))?;
    let _ = daemon.register(service_info);

    // --- Browse for peers ---
    let registry = PeerRegistry::new();
    let browse_receiver = daemon
        .browse(crate::LOCALSEND_SERVICE)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, format!("mdns browse: {e}")))?;

    let reg = registry.clone();
    let callback = on_event.map(Arc::new);
    std::thread::Builder::new()
        .name("ketra-mdns".into())
        .spawn(move || {
            while let Ok(ev) = browse_receiver.recv() {
                match ev {
                    ServiceEvent::ServiceResolved(info) => {
                        if let Some(peer) = build_peer(&info) {
                            let event = PeerEvent::Found(peer.clone());
                            reg.upsert(peer);
                            if let Some(cb) = &callback {
                                cb(event);
                            }
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, service_name) => {
                        reg.remove(&service_name);
                        if let Some(cb) = &callback {
                            cb(PeerEvent::Removed(service_name));
                        }
                    }
                    _ => {}
                }
            }
        })
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    Ok((daemon, registry))
}

/// Build a `Peer` from a resolved mDNS `ServiceInfo`.
fn build_peer(info: &ServiceInfo) -> Option<Peer> {
    let txt = info.get_properties();
    let get = |k: &str| txt.iter().find(|p| p.key().eq_ignore_ascii_case(k)).map(|p| p.val_str().to_string());
    let alias = get("alias")?;
    let fingerprint = get("fingerprint")?;
    let port = get("port").and_then(|p| p.parse::<u16>().ok()).unwrap_or(53317);
    let device_type = get("deviceType")
        .as_deref()
        .and_then(DeviceType::parse)
        .unwrap_or(DeviceType::Desktop);
    let ketra = get("ketra").map(|v| v == "1").unwrap_or(false);
    let ip = info
        .get_addresses()
        .iter()
        .next()
        .copied();
    Some(Peer {
        service_name: info.get_fullname().to_string(),
        alias,
        device_type,
        fingerprint,
        port,
        ip,
        ketra,
    })
}

/// Render a `Peer`'s `DeviceFingerprint` from its hex string (for TLS pinning).
pub fn peer_fingerprint(peer: &Peer) -> DeviceFingerprint {
    // The hex string from mDNS is the raw SHA-256 hex; reconstruct the
    // grouped form for comparison with our pinned verifier.
    let hex = peer.fingerprint.replace('-', "");
    let groups: Vec<String> = hex.as_bytes().chunks(2).map(|c| String::from_utf8_lossy(c).to_string()).collect();
    DeviceFingerprint(groups.join("-"))
}

/// Convenience: convert a `Peer` into a minimal `DeviceInfo` (caller fills
/// the full info via `/info` before transfer).
pub fn peer_to_info(peer: &Peer) -> DeviceInfo {
    DeviceInfo {
        alias: peer.alias.clone(),
        version: 2,
        device_model: String::new(),
        device_type: peer.device_type,
        fingerprint: peer.fingerprint.clone(),
        port: peer.port,
        download: true,
        ketra: if peer.ketra {
            Some(KetraExtension::default())
        } else {
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_upsert_and_snapshot() {
        let reg = PeerRegistry::new();
        let p = Peer {
            service_name: "x._localsend._tcp.local.".into(),
            alias: "X".into(),
            device_type: DeviceType::Desktop,
            fingerprint: "abc".into(),
            port: 53317,
            ip: None,
            ketra: true,
        };
        reg.upsert(p.clone());
        assert_eq!(reg.snapshot().len(), 1);
        reg.remove(&p.service_name);
        assert_eq!(reg.snapshot().len(), 0);
    }

    #[test]
    fn peer_fingerprint_groups_hex() {
        let p = Peer {
            service_name: "x".into(),
            alias: "X".into(),
            device_type: DeviceType::Desktop,
            fingerprint: "abcdef0123456789".into(),
            port: 53317,
            ip: None,
            ketra: false,
        };
        let fp = peer_fingerprint(&p);
        assert_eq!(fp.hex(), "abcdef0123456789");
    }
}

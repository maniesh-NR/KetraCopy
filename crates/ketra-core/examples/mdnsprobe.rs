//! mDNS probe: announce a service for 40s so an external script can
//! unicast/multicast queries at us while `KETRA_MDNS_DEBUG=1` prints
//! every send/recv in the socket loop.
use ketra_core::lan::mdns::Mdns;
use std::collections::BTreeMap;

fn main() {
    let m = match Mdns::spawn() {
        Some(m) => m,
        None => {
            eprintln!("probe: Mdns::spawn failed");
            std::process::exit(1);
        }
    };
    let mut txt = BTreeMap::new();
    txt.insert("alias".to_string(), "probe-box".to_string());
    txt.insert("ketra".to_string(), "1".to_string());
    m.announce(&["_localsend._tcp.local", "_ketra._tcp.local"], "ketra-probe", 53317, &txt);
    eprintln!("probe: announced, idling 40s (debug output follows)…");
    std::thread::sleep(std::time::Duration::from_secs(40));
    m.withdraw();
}

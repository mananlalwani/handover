use mdns_sd::{ServiceDaemon, ServiceInfo};

use crate::errors::NativeError;

/// Registers the `_handover._tcp.local.` advertisement for a bound port and
/// returns the daemon handle, which keeps the record alive while it is held.
pub(crate) fn advertise(port: u16, id: &str) -> Result<ServiceDaemon, NativeError> {
    let mdns = ServiceDaemon::new().map_err(|e| NativeError::Discovery(e.to_string()))?;
    mdns.register(discovery_service(port, id)?)
        .map_err(|e| NativeError::Discovery(e.to_string()))?;
    Ok(mdns)
}

/// Builds the DNS-SD record advertised by [`NativeBackend::run`]. Discovery
/// addresses and TXT values are untrusted hints; trust comes from the TLS
/// certificate fingerprint pinned at pairing time.
pub(crate) fn discovery_service(port: u16, id: &str) -> Result<ServiceInfo, NativeError> {
    let name = format!("Handover-{}", &id[..12]);
    Ok(ServiceInfo::new(
        "_handover._tcp.local.",
        &name,
        &format!("{}.local.", name.to_lowercase()),
        "",
        port,
        &[("v", "1")][..],
    )
    .map_err(|e| NativeError::Discovery(e.to_string()))?
    .enable_addr_auto())
}

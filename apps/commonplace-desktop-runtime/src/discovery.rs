//! Same-network discovery of the local control endpoint (phone-control handoff
//! Part B deliverable 2, path 1: mDNS / DNS-SD).
//!
//! The control endpoint binds LOOPBACK only (see [`crate::control`]); a phone on
//! the SAME LAN cannot reach `127.0.0.1`. This module advertises the *reachable*
//! address of the instance over mDNS/DNS-SD so a same-network phone can discover
//! the Instance URL without the user typing an IP:
//!
//! * [`advertise`] registers a DNS-SD service: a service type
//!   ([`SERVICE_TYPE`]), an instance name, the control port, and a TXT record
//!   carrying a **non-secret** instance id (plus the API version + the loopback
//!   port, both non-secret). mdns-sd's `enable_addr_auto` fills in the host's own
//!   LAN addresses, so the advertised record points at a routable interface, not
//!   loopback. NO device token or pairing code is ever advertised: discovery only
//!   tells a phone *where* the instance is; pairing still gates *access*.
//! * [`browse`] is the discovery side: it browses [`SERVICE_TYPE`] and yields
//!   [`DiscoveredInstance`]s (address, port, instance id) as services resolve.
//!
//! The advertised endpoint is the trust boundary's *front door location*, not the
//! boundary itself: a discovered instance still requires a paired device token on
//! `/v1/*`, exactly as over loopback. The non-secret instance id lets a phone tell
//! two instances apart and remember which one it paired with.
//!
//! mDNS multicast can be blocked (a sandbox, a locked-down corporate LAN). The
//! service/TXT *construction* is pure and unit-tested ([`ServiceAdvertisement`]);
//! the live advertise+browse round-trip is a network test marked `#[ignore]` when
//! multicast is unavailable, surfaced as a degraded path rather than a hard
//! requirement.

use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};

use crate::Result;

/// The DNS-SD service type for the CommonPlace local control endpoint. RFC 6763
/// service type: `_<name>._tcp.<domain>`. `_commonplace-ctl` is the application
/// protocol label; `_tcp.local.` is the standard mDNS link-local TCP domain. The
/// trailing dot is required by mdns-sd's domain-suffix check.
pub const SERVICE_TYPE: &str = "_commonplace-ctl._tcp.local.";

/// TXT-record key carrying the NON-secret instance id (lets a phone distinguish
/// instances and remember the one it paired with). Never a token or pairing code.
pub const TXT_INSTANCE_ID: &str = "instance";
/// TXT-record key carrying the control API version, so a phone can refuse an
/// incompatible instance before pairing.
pub const TXT_API_VERSION: &str = "api";
/// TXT-record key carrying the loopback control port (informational: the relay /
/// local tooling may want it; it is not secret).
pub const TXT_LOOPBACK_PORT: &str = "lport";

/// The control API version advertised in the TXT record. Bump on a breaking
/// change to the `/v1` surface.
pub const CONTROL_API_VERSION: &str = "1";

/// A planned mDNS advertisement: the resolved service type, instance name, host
/// name, port, and TXT properties. Building this is pure (no daemon, no socket),
/// so the service shape + TXT record are unit-testable without multicast. Turn it
/// into a live registration with [`ServiceAdvertisement::register`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ServiceAdvertisement {
    /// DNS-SD service type ([`SERVICE_TYPE`]).
    pub service_type: String,
    /// The instance name (the user-visible "service instance" label, e.g. the
    /// device/host name). DNS-SD escapes this into the full name.
    pub instance_name: String,
    /// The mDNS host name (`<host>.local.`). mdns-sd appends `.local.` if absent.
    pub host_name: String,
    /// The advertised port (the control endpoint's reachable port).
    pub port: u16,
    /// TXT key/value properties. Carries ONLY non-secret discovery metadata
    /// (instance id, api version, loopback port).
    pub txt: Vec<(String, String)>,
}

impl ServiceAdvertisement {
    /// Build the advertisement for a control endpoint.
    ///
    /// * `instance_name`: the user-visible instance label (host/device name).
    /// * `host_name`: the mDNS host name; `.local.` is appended if missing.
    /// * `port`: the reachable control port to advertise.
    /// * `instance_id`: the NON-secret instance id placed in the TXT record.
    /// * `loopback_port`: the loopback control port (informational TXT entry).
    pub fn new(
        instance_name: impl Into<String>,
        host_name: impl Into<String>,
        port: u16,
        instance_id: impl Into<String>,
        loopback_port: u16,
    ) -> Self {
        let txt = vec![
            (TXT_INSTANCE_ID.to_string(), instance_id.into()),
            (TXT_API_VERSION.to_string(), CONTROL_API_VERSION.to_string()),
            (TXT_LOOPBACK_PORT.to_string(), loopback_port.to_string()),
        ];
        Self {
            service_type: SERVICE_TYPE.to_string(),
            instance_name: instance_name.into(),
            host_name: host_name.into(),
            port,
            txt,
        }
    }

    /// The TXT properties as a borrowable map (for assertions / inspection).
    pub fn txt_map(&self) -> HashMap<&str, &str> {
        self.txt
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect()
    }

    /// Construct the mdns-sd [`ServiceInfo`] for this advertisement, with
    /// `enable_addr_auto` so the daemon advertises the host's own LAN addresses
    /// (never loopback). This is the bridge from the pure plan to the crate type;
    /// it does not touch the network. A bad service type / name surfaces as an
    /// error here (e.g. a TXT key containing `=`).
    pub fn to_service_info(&self) -> Result<ServiceInfo> {
        let txt: Vec<(&str, &str)> = self
            .txt
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        // An empty IP list + enable_addr_auto: mdns-sd resolves the host's
        // interface addresses itself, so we never hardcode (or mis-advertise
        // loopback as) the reachable address.
        let info = ServiceInfo::new(
            &self.service_type,
            &self.instance_name,
            &self.host_name,
            "", // addresses filled by enable_addr_auto
            self.port,
            &txt[..],
        )
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
            format!("build mDNS service info: {error}").into()
        })?
        .enable_addr_auto();
        Ok(info)
    }

    /// Register this advertisement on a fresh mDNS daemon and return a live
    /// [`DiscoveryAdvertiser`] handle. Holding the handle keeps the service
    /// advertised; dropping it (or calling [`DiscoveryAdvertiser::shutdown`])
    /// unregisters and stops the daemon.
    ///
    /// This DOES touch the network (multicast). Where multicast is unavailable it
    /// returns an error; callers that treat discovery as best-effort should log
    /// and continue (the relay path still gives cross-network reach).
    pub fn register(&self) -> Result<DiscoveryAdvertiser> {
        let daemon = ServiceDaemon::new()
            .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
                format!("start mDNS daemon: {error}").into()
            })?;
        let info = self.to_service_info()?;
        let fullname = info.get_fullname().to_string();
        daemon
            .register(info)
            .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
                format!("register mDNS service: {error}").into()
            })?;
        Ok(DiscoveryAdvertiser {
            daemon: Some(daemon),
            fullname,
        })
    }
}

/// A live mDNS advertisement. The service is advertised for as long as this is
/// held. `shutdown` (or `Drop`) unregisters it and stops the daemon.
pub struct DiscoveryAdvertiser {
    daemon: Option<ServiceDaemon>,
    /// The registered service's full name (for unregister + diagnostics).
    fullname: String,
}

impl DiscoveryAdvertiser {
    /// The full DNS-SD name the service was registered under.
    pub fn fullname(&self) -> &str {
        &self.fullname
    }

    /// Unregister the service and stop the daemon. Best-effort: errors are
    /// returned but the daemon is dropped regardless.
    pub fn shutdown(mut self) -> Result<()> {
        Self::teardown(&mut self)
    }

    fn teardown(advertiser: &mut DiscoveryAdvertiser) -> Result<()> {
        if let Some(daemon) = advertiser.daemon.take() {
            // unregister returns a status receiver; we do not block on it (the
            // daemon shutdown below tears everything down anyway).
            let _ = daemon.unregister(&advertiser.fullname);
            daemon
                .shutdown()
                .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
                    format!("shutdown mDNS daemon: {error}").into()
                })?;
        }
        Ok(())
    }
}

impl Drop for DiscoveryAdvertiser {
    fn drop(&mut self) {
        if self.daemon.is_some() {
            if let Err(error) = Self::teardown(self) {
                eprintln!("commonplace-desktop-runtime: mDNS advertiser shutdown failed: {error}");
            }
        }
    }
}

/// One instance discovered on the LAN: its reachable addresses, port, and the
/// non-secret instance id from the TXT record. A phone turns the first address +
/// port into an `Instance URL` and then pairs over it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscoveredInstance {
    /// The full DNS-SD name of the resolved service.
    pub fullname: String,
    /// The host name the service resolved to.
    pub host_name: String,
    /// Reachable IP addresses for the instance (LAN addresses, not loopback).
    pub addresses: Vec<IpAddr>,
    /// The advertised control port.
    pub port: u16,
    /// The non-secret instance id from the TXT record, if present.
    pub instance_id: Option<String>,
    /// The advertised control API version from the TXT record, if present.
    pub api_version: Option<String>,
}

impl DiscoveredInstance {
    /// The first reachable address as an `http://<ip>:<port>` base URL, if any.
    /// This is the `Instance URL` a phone would pair against.
    pub fn http_base_url(&self) -> Option<String> {
        self.addresses.first().map(|addr| match addr {
            // Bracket IPv6 literals for a valid URL authority.
            IpAddr::V6(v6) => format!("http://[{v6}]:{}", self.port),
            IpAddr::V4(v4) => format!("http://{v4}:{}", self.port),
        })
    }
}

/// Browse the LAN for CommonPlace control endpoints for up to `timeout`,
/// returning every instance that resolved in that window. Blocking (mdns-sd's
/// browse is sync over a flume channel); call it on a blocking thread/task.
///
/// This is the discovery counterpart to [`advertise`]. It does NOT pair or
/// authenticate -- it only locates instances; access still requires a device
/// token. Multicast-unavailable environments return an empty list (no resolves
/// arrive) rather than an error, so a caller can treat "found nothing" and
/// "discovery blocked" the same degraded way.
pub fn browse(timeout: Duration) -> Result<Vec<DiscoveredInstance>> {
    let daemon = ServiceDaemon::new()
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
            format!("start mDNS daemon for browse: {error}").into()
        })?;
    let receiver = daemon
        .browse(SERVICE_TYPE)
        .map_err(|error| -> Box<dyn std::error::Error + Send + Sync> {
            format!("browse mDNS {SERVICE_TYPE}: {error}").into()
        })?;

    let deadline = std::time::Instant::now() + timeout;
    let mut found: HashMap<String, DiscoveredInstance> = HashMap::new();
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            break;
        }
        match receiver.recv_timeout(remaining) {
            Ok(ServiceEvent::ServiceResolved(resolved)) => {
                let instance = resolved_to_instance(&resolved);
                found.insert(instance.fullname.clone(), instance);
            }
            // Other events (search started/found/removed/stopped) are not
            // resolutions; keep waiting for resolves until the deadline.
            Ok(_) => {}
            // No more events within the window: done.
            Err(_) => break,
        }
    }
    // Best-effort stop; the browse is done regardless.
    let _ = daemon.shutdown();
    Ok(found.into_values().collect())
}

/// Map a resolved mdns-sd service to our [`DiscoveredInstance`]. Reads the TXT
/// instance id / api version (non-secret) and the resolved addresses + port.
fn resolved_to_instance(resolved: &mdns_sd::ResolvedService) -> DiscoveredInstance {
    let addresses: Vec<IpAddr> = resolved
        .get_addresses()
        .iter()
        .map(|scoped| scoped.to_ip_addr())
        .collect();
    DiscoveredInstance {
        fullname: resolved.get_fullname().to_string(),
        host_name: resolved.get_hostname().to_string(),
        addresses,
        port: resolved.get_port(),
        instance_id: resolved
            .get_property_val_str(TXT_INSTANCE_ID)
            .map(str::to_string),
        api_version: resolved
            .get_property_val_str(TXT_API_VERSION)
            .map(str::to_string),
    }
}

/// Advertise a control endpoint over mDNS. Convenience wrapper over
/// [`ServiceAdvertisement::new`] + [`ServiceAdvertisement::register`]; returns a
/// [`DiscoveryAdvertiser`] to hold for the lifetime of the advertisement.
///
/// `instance_name` is the user-visible label; `host_name` is the mDNS host
/// (`.local.` appended if missing); `port` is the reachable control port;
/// `instance_id` is the NON-secret id placed in the TXT record; `loopback_port`
/// is the (non-secret) loopback control port for informational purposes.
pub fn advertise(
    instance_name: impl Into<String>,
    host_name: impl Into<String>,
    port: u16,
    instance_id: impl Into<String>,
    loopback_port: u16,
) -> Result<DiscoveryAdvertiser> {
    ServiceAdvertisement::new(instance_name, host_name, port, instance_id, loopback_port).register()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn advertisement_carries_nonsecret_txt_only() {
        let advert = ServiceAdvertisement::new(
            "Travis Desktop",
            "travis-mbp",
            17888,
            "inst_abc123",
            17888,
        );
        assert_eq!(advert.service_type, SERVICE_TYPE);
        assert_eq!(advert.instance_name, "Travis Desktop");
        assert_eq!(advert.port, 17888);

        let txt = advert.txt_map();
        assert_eq!(txt.get(TXT_INSTANCE_ID), Some(&"inst_abc123"));
        assert_eq!(txt.get(TXT_API_VERSION), Some(&CONTROL_API_VERSION));
        assert_eq!(txt.get(TXT_LOOPBACK_PORT), Some(&"17888"));

        // The TXT record must NOT carry anything that looks like a credential.
        // (Discovery is location-only; pairing still gates access.) Assert no key
        // or value resembles a token/secret/pairing code.
        for (key, value) in &advert.txt {
            let lower_key = key.to_ascii_lowercase();
            assert!(
                !lower_key.contains("token")
                    && !lower_key.contains("secret")
                    && !lower_key.contains("pairing")
                    && !lower_key.contains("code"),
                "TXT key {key} must not name a credential"
            );
            // The non-secret instance id is short + opaque; a 64-hex device
            // secret would be 64 chars. Guard against accidentally publishing one.
            assert!(
                value.len() < 48,
                "TXT value for {key} is suspiciously long (possible secret leak)"
            );
        }
    }

    #[test]
    fn service_info_builds_from_advertisement() {
        // The pure plan -> mdns-sd ServiceInfo bridge must succeed and preserve
        // the type/port/TXT (no network involved).
        let advert =
            ServiceAdvertisement::new("Host", "myhost", 50071, "inst_xyz", 17888);
        let info = advert
            .to_service_info()
            .expect("service info builds from a valid advertisement");
        assert_eq!(info.get_type(), SERVICE_TYPE);
        // mdns-sd appends `.local.` to a bare host name.
        assert!(
            info.get_hostname().contains("myhost"),
            "host name preserved: {}",
            info.get_hostname()
        );
        assert_eq!(
            info.get_property_val_str(TXT_INSTANCE_ID),
            Some("inst_xyz"),
            "the non-secret instance id is in the service TXT record"
        );
        assert_eq!(info.get_property_val_str(TXT_API_VERSION), Some("1"));
    }

    #[test]
    fn discovered_instance_builds_an_http_base_url() {
        let v4 = DiscoveredInstance {
            fullname: "Host._commonplace-ctl._tcp.local.".to_string(),
            host_name: "host.local.".to_string(),
            addresses: vec![IpAddr::from([192, 168, 1, 42])],
            port: 50071,
            instance_id: Some("inst_xyz".to_string()),
            api_version: Some("1".to_string()),
        };
        assert_eq!(
            v4.http_base_url().as_deref(),
            Some("http://192.168.1.42:50071")
        );

        // No address resolved yet -> no URL (a phone keeps browsing).
        let pending = DiscoveredInstance {
            addresses: vec![],
            ..v4.clone()
        };
        assert_eq!(pending.http_base_url(), None);

        // IPv6 literals are bracketed for a valid authority.
        let v6 = DiscoveredInstance {
            addresses: vec!["fe80::1".parse().unwrap()],
            ..v4
        };
        assert_eq!(v6.http_base_url().as_deref(), Some("http://[fe80::1]:50071"));
    }

    // Live mDNS round-trip: advertise a service, browse for it on a SECOND daemon,
    // and assert we discover it with the right port + TXT instance id. This needs
    // working multicast on the loopback/LAN interface. In a sandbox that blocks
    // multicast no resolution arrives; rather than fail the build, this is
    // `#[ignore]`d and the pure construction tests above are the always-on
    // coverage. Run explicitly with:
    //   cargo test -p commonplace-desktop-runtime --lib discovery -- --ignored
    #[test]
    #[ignore = "requires working mDNS multicast (often blocked in CI/sandbox); pure construction is covered by the non-network tests"]
    fn live_advertise_then_browse_finds_the_service() {
        let instance_id = format!("inst_{}", crate::pairing::random_token_hex(4).unwrap());
        let host = format!("cpdr-test-{}", crate::pairing::random_token_hex(2).unwrap());
        let _advertiser = advertise(
            "CPDR Test Instance",
            host,
            50071,
            instance_id.clone(),
            17888,
        )
        .expect("advertise over mDNS");

        // Browse on a fresh daemon (separate from the advertiser's) and look for
        // our instance id. Give it a generous window for the resolve.
        let found = browse(Duration::from_secs(8)).expect("browse mDNS");
        let ours = found
            .iter()
            .find(|inst| inst.instance_id.as_deref() == Some(instance_id.as_str()));
        assert!(
            ours.is_some(),
            "browse should discover the advertised instance (found {} services: {:?})",
            found.len(),
            found.iter().map(|f| &f.fullname).collect::<Vec<_>>()
        );
        let ours = ours.unwrap();
        assert_eq!(ours.port, 50071, "the advertised control port is discovered");
        assert_eq!(ours.api_version.as_deref(), Some("1"));
    }
}

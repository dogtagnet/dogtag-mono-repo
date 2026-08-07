//! Does THIS machine still answer at the address the QR names?
//!
//! `DEPLOYMENT_URL` is baked at boot (demo-up.sh stamps the LAN IP), so when the machine's address
//! changes underneath a running vet, every QR already on screen points at an address nobody is on —
//! the owner's phone posts into the void, the vet's log records nothing, and the operator is shown
//! a timeout for a connection failure. Measured live 2026-08-07: the Mac moved from 192.168.1.71 to
//! 192.168.16.117 and the portal said "expired".
//!
//! The check is deliberately NARROW: it establishes only whether an IP-literal QR host is still one
//! of this machine's own addresses, via the OS routing table — `UdpSocket::connect` performs a local
//! route lookup (no packet is sent) and `local_addr` reports the source address the OS would use to
//! reach the target. Connecting toward an address this machine OWNS selects that address itself;
//! toward anything else it selects the current outbound address, which is then reported as the
//! address the operator should expect QRs to carry after a restart.
//!
//! What it cannot check, it says so instead of half-doing: a HOSTNAME (a tunnel, a real deployment)
//! resolves on the PHONE's resolver, not here, so whether phones can reach it is genuinely not
//! establishable from this machine — that is `Unknown` with the reason, never a verdict. Same for a
//! failed route lookup. Could-not-check is never reported as either "reachable" or "dead".

use serde_json::json;

/// The three-way answer. `NotSelfAddressed` carries the address this machine WOULD use — the one a
/// freshly restarted stack would stamp into QRs — so the operator's remedy names a concrete value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelfAddressCheck {
    /// The QR names an IP this machine still owns. Claims ONLY that — not that phones can route to
    /// it (a VPN-only address passes), and not that the port is listening.
    SelfAddressed,
    /// The QR names an IP this machine no longer owns: phones sending to it cannot reach this
    /// backend. `current_address` is where this machine answers today.
    NotSelfAddressed { current_address: String },
    /// Could not be established from this machine; `detail` says why.
    Unknown { detail: String },
}

/// Extract the host from a URL-shaped string without a URL crate: strip the scheme, take the
/// authority, strip the port (`[v6]` brackets handled). Returns `None` for an empty host.
pub fn host_of(url: &str) -> Option<String> {
    let rest = match url.split_once("://") {
        Some((_, r)) => r,
        None => url,
    };
    let authority = rest.split('/').next().unwrap_or("");
    let host = if let Some(stripped) = authority.strip_prefix('[') {
        stripped.split(']').next().unwrap_or("")
    } else {
        authority.split(':').next().unwrap_or("")
    };
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

/// Run the self-address check for one host (as extracted by [`host_of`]).
pub fn self_address_check(host: &str) -> SelfAddressCheck {
    let ip: std::net::IpAddr = match host.parse() {
        Ok(ip) => ip,
        Err(_) => {
            return SelfAddressCheck::Unknown {
                detail: "the QR names a hostname, not an IP; whether phones can resolve and \
                         reach it cannot be established from this machine"
                    .to_string(),
            }
        }
    };
    let bind_addr: (&str, u16) = if ip.is_ipv4() { ("0.0.0.0", 0) } else { ("::", 0) };
    // UDP connect = a route lookup, no packet. Port 9 (discard) is arbitrary; nothing is sent.
    let local = std::net::UdpSocket::bind(bind_addr)
        .and_then(|s| s.connect((ip, 9)).map(|_| s))
        .and_then(|s| s.local_addr());
    match local {
        Ok(local) if local.ip() == ip => SelfAddressCheck::SelfAddressed,
        Ok(local) => SelfAddressCheck::NotSelfAddressed {
            current_address: local.ip().to_string(),
        },
        Err(e) => SelfAddressCheck::Unknown {
            detail: format!("route lookup failed: {e}"),
        },
    }
}

/// The JSON block the issuance-session responses carry beside the QR: which host the QR names and
/// whether this machine still answers there. `check` is `selfAddressed` / `notSelfAddressed` /
/// `unknown`; `currentAddress` is present only on a definite mismatch, `detail` only on `unknown`.
pub fn qr_address_json(deployment_url: &str) -> serde_json::Value {
    let host = match host_of(deployment_url) {
        Some(h) => h,
        None => {
            return json!({
                "host": "",
                "check": "unknown",
                "detail": "DEPLOYMENT_URL carries no host",
            })
        }
    };
    match self_address_check(&host) {
        SelfAddressCheck::SelfAddressed => json!({ "host": host, "check": "selfAddressed" }),
        SelfAddressCheck::NotSelfAddressed { current_address } => json!({
            "host": host,
            "check": "notSelfAddressed",
            "currentAddress": current_address,
        }),
        SelfAddressCheck::Unknown { detail } => json!({
            "host": host,
            "check": "unknown",
            "detail": detail,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_of_strips_scheme_port_and_path() {
        assert_eq!(host_of("http://192.168.1.71:41874/p/x"), Some("192.168.1.71".into()));
        assert_eq!(host_of("https://vet.example.com/api"), Some("vet.example.com".into()));
        assert_eq!(host_of("http://[::1]:41874/p/x"), Some("::1".into()));
        assert_eq!(host_of("192.168.1.71:41874"), Some("192.168.1.71".into()));
        assert_eq!(host_of("http:///p/x"), None);
        assert_eq!(host_of(""), None);
    }

    #[test]
    fn loopback_is_always_self_addressed() {
        // 127.0.0.1 is owned by every machine: the route lookup must select it as its own source.
        assert_eq!(self_address_check("127.0.0.1"), SelfAddressCheck::SelfAddressed);
    }

    /// The load-bearing direction: an address this machine cannot own must NEVER be claimed as
    /// self-addressed. 192.0.2.1 is TEST-NET-1 (RFC 5737), reserved for documentation and never
    /// assigned, so on a machine with a default route the check answers NotSelfAddressed with the
    /// real current address; with no route at all it answers Unknown. Either is honest — a
    /// confident `SelfAddressed` claim would be the false pass this module exists to remove.
    #[test]
    fn an_unowned_address_is_never_reported_as_self_addressed() {
        match self_address_check("192.0.2.1") {
            SelfAddressCheck::SelfAddressed => {
                panic!("TEST-NET-1 cannot be a local address; a self-addressed claim is a false pass")
            }
            SelfAddressCheck::NotSelfAddressed { current_address } => {
                assert!(!current_address.is_empty(), "the remedy names the current address");
                assert_ne!(current_address, "192.0.2.1");
            }
            SelfAddressCheck::Unknown { detail } => {
                assert!(!detail.is_empty(), "an unknown carries its reason");
            }
        }
    }

    #[test]
    fn a_hostname_is_unknown_with_the_reason_never_a_verdict() {
        match self_address_check("vet.example.com") {
            SelfAddressCheck::Unknown { detail } => {
                assert!(detail.contains("hostname"), "{detail}");
            }
            other => panic!("a hostname cannot be route-checked from here: {other:?}"),
        }
    }

    #[test]
    fn the_json_block_never_claims_a_mismatch_without_naming_the_current_address() {
        let v = qr_address_json("http://192.0.2.1:41874");
        assert_eq!(v["host"], "192.0.2.1");
        match v["check"].as_str().unwrap() {
            "notSelfAddressed" => {
                assert!(v["currentAddress"].as_str().is_some_and(|a| !a.is_empty()))
            }
            "unknown" => assert!(v["detail"].as_str().is_some_and(|d| !d.is_empty())),
            other => panic!("TEST-NET-1 must not read as owned: {other}"),
        }
    }
}

//! Fail-closed production startup guards (audit H2).
//!
//! In production mode (neither `DEMO_MODE` nor `VITE_DEMO_MODE` set), the central/admin server MUST
//! NOT boot with an unset/dev-default `ADMIN_PASSWORD` or an unset `ADMIN_PRIVATE_KEY` — those guard
//! issuer whitelisting, GDPR erasure, and every on-chain admin write. The local/demo path keeps its
//! convenient defaults.
//!
//! The validation itself is a pure function so it is unit-testable without mutating process env.

/// A secret that must be a real, operator-supplied value before a production boot.
pub struct SecretSpec<'a> {
    pub name: &'a str,
    pub value: &'a str,
    /// The insecure default this secret falls back to in demo/local mode (empty = "must be set").
    pub dev_default: &'a str,
}

/// True when the process is running in demo/local mode: either `DEMO_MODE` or `VITE_DEMO_MODE` is set
/// to a non-empty, non-`0`/`false` value. Production = neither flag set (matches the README's
/// `VITE_DEMO_MODE` set = demo, unset = production convention).
pub fn is_demo_mode() -> bool {
    ["DEMO_MODE", "VITE_DEMO_MODE"].iter().any(|k| {
        std::env::var(k)
            .ok()
            .map(|v| {
                let v = v.trim().to_ascii_lowercase();
                !v.is_empty() && v != "0" && v != "false"
            })
            .unwrap_or(false)
    })
}

/// Fail-closed secret validation. In demo mode this is always `Ok`. In production every secret must be
/// non-empty and not equal to its dev default; otherwise returns a descriptive error naming every
/// offending secret so the operator can fix them all in one go.
pub fn validate_production_secrets(demo: bool, secrets: &[SecretSpec]) -> Result<(), String> {
    if demo {
        return Ok(());
    }
    let mut bad = Vec::new();
    for s in secrets {
        if s.value.trim().is_empty() {
            bad.push(format!("{} is unset/empty", s.name));
        } else if s.value == s.dev_default {
            bad.push(format!("{} is set to the insecure dev default", s.name));
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "refusing to boot in production mode: {}. Provide real secrets via environment before \
             deploying, or set DEMO_MODE=1 for local/demo.",
            bad.join("; ")
        ))
    }
}

// ------------------------------------------------------------------------------------------------
// Control-plane authority preflight.
//
// Every privileged write routes through `GovernanceAction`, which flips from "executed" to
// "proposed" when the hosted signer does not hold the required authority. That flip is CORRECT for the
// governance Phase-2 split (DEFAULT_ADMIN legitimately lives on the governance signer, so those actions
// are meant to be proposed for out-of-band signing) — but it is indistinguishable from booting with
// the WRONG KEY entirely. A retired deployer EOA that lost WHITELIST_ADMIN yields a stack that starts
// cleanly and reports success, while every grant returns disposition:"proposed" with unsigned calldata
// and NOTHING lands on-chain.
//
// This preflight resolves the authorities once at boot and names what is missing, so an unintended
// unauthorized signer is loud instead of silent. The verdict is a pure function over already-read
// holdings so it is unit-testable without a chain.
// ------------------------------------------------------------------------------------------------

/// One control-plane authority, together with whether the hosted signer was observed to hold it.
pub struct AuthorityCheck<'a> {
    /// Human name of the authority, e.g. `WHITELIST_ADMIN`.
    pub name: &'a str,
    /// The contract the authority is read from.
    pub target: &'a str,
    /// What holding it lets the control plane do.
    pub capability: &'a str,
    /// `Some(true/false)` when resolved; `None` when unreadable (RPC failure, or target unconfigured)
    /// — an unreadable authority is reported as unknown rather than counted as missing.
    pub held: Option<bool>,
}

/// The outcome of the boot-time authority preflight.
#[derive(Debug, PartialEq, Eq)]
pub enum AuthorityVerdict {
    /// No authority could be resolved at all (unreachable RPC / nothing configured).
    Unknown,
    /// The hosted signer holds every authority that could be read.
    AllHeld,
    /// It holds some, not all. The rest legitimately belong elsewhere (e.g. Phase-2 DEFAULT_ADMIN) and
    /// will be returned as proposals for out-of-band signing.
    PartiallyHeld { missing: Vec<String> },
    /// It holds NONE of the readable authorities. Every privileged write will degrade to an unsigned
    /// proposal, so the stack looks healthy while nothing reaches the chain. Almost always the wrong key.
    HoldsNothing { missing: Vec<String> },
}

impl AuthorityVerdict {
    /// True when this verdict means "the hosted key cannot perform ANY privileged write" — the
    /// silent-degradation case a boot should shout about.
    pub fn is_unauthorized(&self) -> bool {
        matches!(self, AuthorityVerdict::HoldsNothing { .. })
    }
}

/// Resolve the preflight verdict from the observed holdings.
pub fn authority_verdict(checks: &[AuthorityCheck]) -> AuthorityVerdict {
    let readable: Vec<&AuthorityCheck> = checks.iter().filter(|c| c.held.is_some()).collect();
    if readable.is_empty() {
        return AuthorityVerdict::Unknown;
    }
    let missing: Vec<String> = readable
        .iter()
        .filter(|c| c.held == Some(false))
        .map(|c| format!("{} on {} ({})", c.name, c.target, c.capability))
        .collect();
    if missing.is_empty() {
        AuthorityVerdict::AllHeld
    } else if missing.len() == readable.len() {
        AuthorityVerdict::HoldsNothing { missing }
    } else {
        AuthorityVerdict::PartiallyHeld { missing }
    }
}

/// The operator-facing preflight message. Names the hosted signer and every missing authority, and
/// spells out the consequence, so a wrong-key boot cannot be mistaken for a healthy one.
pub fn authority_preflight_message(hosted: Option<&str>, verdict: &AuthorityVerdict) -> String {
    let who = hosted.unwrap_or("<no signer registered>");
    match verdict {
        AuthorityVerdict::Unknown => format!(
            "control-plane authority preflight: could not resolve ANY authority for hosted signer \
             {who} (RPC unreachable, or FACTORY_ADDR/ISSUER_REGISTRY_ADDR unconfigured). Privileged \
             writes may silently degrade to unsigned proposals."
        ),
        AuthorityVerdict::AllHeld => format!(
            "control-plane authority preflight: hosted signer {who} holds every configured authority"
        ),
        AuthorityVerdict::PartiallyHeld { missing } => format!(
            "control-plane authority preflight: hosted signer {who} does NOT hold {}. Actions gated by \
             {} will return disposition=\"proposed\" with unsigned calldata for the holder to execute \
             out-of-band (expected after the governance Phase-2 handover).",
            missing.join("; "),
            if missing.len() == 1 { "it" } else { "them" },
        ),
        AuthorityVerdict::HoldsNothing { missing } => format!(
            "control-plane authority preflight FAILED: hosted signer {who} holds NONE of the \
             control-plane authorities - missing {}. EVERY privileged write (whitelist grants, issuer \
             approval, factory deploys) will return disposition=\"proposed\" with unsigned calldata and \
             NOTHING WILL LAND ON-CHAIN, while the stack otherwise looks healthy. This is almost \
             always the wrong key: a retired deployer EOA whose roles moved to the governance signer. \
             Set ADMIN_PRIVATE_KEY to the key that holds these authorities (for the demo stack: \
             GOVERNANCE_PRIVATE_KEY). Set ADMIN_REQUIRE_AUTHORITY=1 to refuse to boot in this state.",
            missing.join("; "),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn check<'a>(name: &'a str, held: Option<bool>) -> AuthorityCheck<'a> {
        AuthorityCheck {
            name,
            target: "0xreg",
            capability: "cap",
            held,
        }
    }

    #[test]
    fn all_held_is_clean() {
        let c = [check("WHITELIST_ADMIN", Some(true)), check("OWNER", Some(true))];
        assert_eq!(authority_verdict(&c), AuthorityVerdict::AllHeld);
        assert!(!authority_verdict(&c).is_unauthorized());
    }

    /// The Phase-2 split: WHITELIST_ADMIN stays on the hosted key, DEFAULT_ADMIN legitimately moved to
    /// governance. That must stay a warning, NOT an error — proposing is the intended behaviour here.
    #[test]
    fn partial_holding_is_the_legitimate_phase2_split() {
        let c = [
            check("WHITELIST_ADMIN", Some(true)),
            check("DEFAULT_ADMIN", Some(false)),
        ];
        let v = authority_verdict(&c);
        assert!(matches!(v, AuthorityVerdict::PartiallyHeld { .. }));
        assert!(!v.is_unauthorized(), "a legitimate split must not read as unauthorized");
        let msg = authority_preflight_message(Some("0xabc"), &v);
        assert!(msg.contains("DEFAULT_ADMIN"), "{msg}");
        assert!(msg.contains("out-of-band"), "{msg}");
    }

    /// The bug: booting with the retired deployer EOA that holds nothing. Must be unmistakable.
    #[test]
    fn holding_nothing_is_flagged_unauthorized_and_names_every_authority() {
        let c = [
            check("WHITELIST_ADMIN", Some(false)),
            check("DEFAULT_ADMIN", Some(false)),
            check("FACTORY_OWNER", Some(false)),
        ];
        let v = authority_verdict(&c);
        assert!(v.is_unauthorized());
        let msg = authority_preflight_message(Some("0xdead"), &v);
        for name in ["WHITELIST_ADMIN", "DEFAULT_ADMIN", "FACTORY_OWNER"] {
            assert!(msg.contains(name), "message must name {name}: {msg}");
        }
        assert!(msg.contains("0xdead"), "must name the offending signer: {msg}");
        assert!(msg.contains("NOTHING WILL LAND ON-CHAIN"), "{msg}");
    }

    /// An unreadable authority is unknown, never silently counted as held or missing.
    #[test]
    fn unreadable_authorities_do_not_manufacture_a_verdict() {
        let c = [check("WHITELIST_ADMIN", None), check("DEFAULT_ADMIN", None)];
        assert_eq!(authority_verdict(&c), AuthorityVerdict::Unknown);
        // A single readable+missing authority alongside unreadable ones still reports unauthorized.
        let c2 = [check("WHITELIST_ADMIN", Some(false)), check("DEFAULT_ADMIN", None)];
        assert!(authority_verdict(&c2).is_unauthorized());
    }

    fn spec<'a>(name: &'a str, value: &'a str, def: &'a str) -> SecretSpec<'a> {
        SecretSpec {
            name,
            value,
            dev_default: def,
        }
    }

    #[test]
    fn demo_mode_skips_all_checks() {
        let s = [spec("ADMIN_PASSWORD", "admin-pw", "admin-pw")];
        assert!(validate_production_secrets(true, &s).is_ok());
    }

    #[test]
    fn production_rejects_dev_default_password() {
        let s = [spec("ADMIN_PASSWORD", "admin-pw", "admin-pw")];
        let err = validate_production_secrets(false, &s).unwrap_err();
        assert!(err.contains("ADMIN_PASSWORD"), "{err}");
    }

    #[test]
    fn production_rejects_unset_private_key() {
        // ADMIN_PRIVATE_KEY has no dev default: dev_default = "" means "must be set".
        let s = [spec("ADMIN_PRIVATE_KEY", "", "")];
        let err = validate_production_secrets(false, &s).unwrap_err();
        assert!(err.contains("ADMIN_PRIVATE_KEY"), "{err}");
    }

    #[test]
    fn production_accepts_real_secrets() {
        let s = [
            spec("ADMIN_PASSWORD", "s3cret-admin", "admin-pw"),
            spec("ADMIN_PRIVATE_KEY", "0xabc123", ""),
        ];
        assert!(validate_production_secrets(false, &s).is_ok());
    }
}

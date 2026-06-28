//! Fail-closed production startup guards (audit H2 + M1).
//!
//! In production mode (neither `DEMO_MODE` nor `VITE_DEMO_MODE` set), the server MUST NOT boot with
//! unset or well-known dev-default secrets — those guard issuance, custody unlock, and the
//! cross-backend HMAC. The local/demo path (either flag set) keeps its convenient defaults.
//!
//! The validation itself is a pure function so it is unit-testable without mutating process env.

/// A secret that must be a real, operator-supplied value before a production boot.
pub struct SecretSpec<'a> {
    pub name: &'a str,
    pub value: &'a str,
    /// The insecure default this secret falls back to in demo/local mode.
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

#[cfg(test)]
mod tests {
    use super::*;

    fn spec<'a>(name: &'a str, value: &'a str, def: &'a str) -> SecretSpec<'a> {
        SecretSpec { name, value, dev_default: def }
    }

    #[test]
    fn demo_mode_skips_all_checks() {
        // Even all-default secrets are fine in demo mode.
        let s = [spec("OPERATOR_PASSWORD", "operator-dev-password", "operator-dev-password")];
        assert!(validate_production_secrets(true, &s).is_ok());
    }

    #[test]
    fn production_rejects_dev_defaults() {
        let s = [
            spec("OPERATOR_PASSWORD", "operator-dev-password", "operator-dev-password"),
            spec("CENTRAL_HMAC_SECRET", "a-real-secret", "dev-central-hmac-secret"),
        ];
        let err = validate_production_secrets(false, &s).unwrap_err();
        assert!(err.contains("OPERATOR_PASSWORD"), "{err}");
        // The real secret must NOT be flagged.
        assert!(!err.contains("CENTRAL_HMAC_SECRET"), "{err}");
    }

    #[test]
    fn production_rejects_empty_or_whitespace() {
        let s = [spec("ADMIN_PASSWORD", "   ", "admin-dev-password")];
        let err = validate_production_secrets(false, &s).unwrap_err();
        assert!(err.contains("ADMIN_PASSWORD"), "{err}");
    }

    #[test]
    fn production_accepts_real_secrets() {
        let s = [
            spec("OPERATOR_PASSWORD", "s3cret-op", "operator-dev-password"),
            spec("ADMIN_PASSWORD", "s3cret-admin", "admin-dev-password"),
            spec("CENTRAL_HMAC_SECRET", "s3cret-hmac", "dev-central-hmac-secret"),
        ];
        assert!(validate_production_secrets(false, &s).is_ok());
    }
}

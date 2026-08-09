//! Deployment lineage: a dog tag's identity is (deployment, id), and an issuer clone is only an
//! issuer OF a deployment — never of the product at large.
//!
//! The fault this closes was measured live (2026-08-09): after a redeploy, `contracts/.env` still
//! named the previous generation's `PROFILE_ISSUER_ADDR` — the env sync DELIBERATELY preserves the
//! per-provider clone variables, because the ledger does not own them — while `FACTORY_ADDR`,
//! `SBT_CONSENT_ADDR` and `VERIFICATION_REGISTRY_CONSENT_ADDR` all moved with the ledger. The vet
//! happily `issue(R)`'d into the old-generation clone and `mintCustodial`'d into the NEW SBT, and
//! the result was a credential no verifier can attribute: integrity VALID, `isValid` yes, and the
//! mandatory issuer-whitelist pillar permanently UNRESOLVED, because the new factory's write-once
//! `rootIssuer[R]` never heard of the root. Nothing refused the mixed-generation issuance; the
//! honest verifier verdict arrived only after the damage was irreversible (`registerRoot` and
//! `profileRoot` are both write-once).
//!
//! The check asks the chain's own write-once facts, never a second copy of the configuration:
//!
//!   * `factory.isClone(clone)` — the factory's own storage, filled only by `createIssuer`. A
//!     definite `false` from the CONFIGURED factory is proof the clone belongs to a different
//!     deployment.
//!   * `verificationRegistry.rootIndex()` / `.sbt()` — the registry's two immutable pins. These
//!     anchor "the deployment in use" to the contract that DEFINES attribution, so a configured
//!     factory or SBT that disagrees with them is itself a mixed configuration, however each
//!     address got into the environment.
//!
//! It deliberately does NOT compare `clone.registry()` against `ISSUER_REGISTRY_ADDR`: the
//! issuance preflight sources its authority from the clone on purpose (see
//! [`crate::chain::ChainClient::issuance_capability`]), and re-coupling that variable to the
//! issuance axis is the exact drift that was removed.
//!
//! VERDICT DISCIPLINE, same as every gate in this backend: only a DEFINITE mismatch refuses;
//! could-not-check warns and never blocks (the point-of-use preflights still stand). The refusal
//! runs where the operator can act — boot (a log), session start and `credentials/prepare`
//! (before anything is allocated), the bind and the retry (before the one-time token or the
//! chain is touched).

use crate::chain::ChainClient;
use crate::verify::valid_contract_addr;
use std::time::Duration;

/// One verdict per configured clone. `Foreign` is the only refusal; its string names the foreign
/// contract, the deployment fact it disagrees with, and the fix.
#[derive(Debug, Clone, PartialEq)]
pub enum DeploymentLineage {
    /// Every establishable read succeeded and agreed: the clone was deployed by the configured
    /// factory, and (where a verification registry is configured) that factory and the configured
    /// SBT are the ones the registry pins.
    Member,
    /// A definite mismatch — the clone, or the configuration itself, mixes deployments.
    Foreign(String),
    /// Could not check. Never a refusal, and never rendered as a pass: callers surface the detail
    /// as a warning.
    Unknown(String),
}

/// The three configured addresses that name "the deployment in use". Split out of `Config` so the
/// unit tests here need no full `Config` literal.
pub struct DeploymentRefs<'a> {
    pub factory_addr: &'a str,
    pub verification_registry_addr: &'a str,
    pub sbt_addr: &'a str,
}

impl<'a> DeploymentRefs<'a> {
    pub fn of(cfg: &'a crate::app::Config) -> Self {
        Self {
            factory_addr: &cfg.factory_addr,
            verification_registry_addr: &cfg.verification_registry_consent_addr,
            sbt_addr: &cfg.sbt_consent_addr,
        }
    }
}

fn same_addr(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Bound on the whole assessment, mirroring `mint_role_gate`: the diagnostic must never gate
/// liveness, and a hung RPC answers `Unknown`, not a refusal.
const LINEAGE_TIMEOUT: Duration = Duration::from_secs(3);

/// Does `clone_addr` (configured as `env_var`) belong to the same deployment as the factory, SBT
/// and verification registry in use?
pub async fn issuer_clone_lineage(
    chain: &dyn ChainClient,
    refs: &DeploymentRefs<'_>,
    clone_addr: &str,
    env_var: &str,
) -> DeploymentLineage {
    let factory = refs.factory_addr;
    if !valid_contract_addr(factory) {
        return DeploymentLineage::Unknown(format!(
            "no FACTORY_ADDR is configured, so whether {env_var} {clone_addr} belongs to this \
             deployment could not be established — a clone left over from a superseded deployment \
             would not be refused here"
        ));
    }
    let work = assess(chain, refs, factory, clone_addr, env_var);
    match tokio::time::timeout(LINEAGE_TIMEOUT, work).await {
        Ok(v) => v,
        Err(_) => DeploymentLineage::Unknown(format!(
            "the deployment-lineage reads for {env_var} {clone_addr} timed out"
        )),
    }
}

async fn assess(
    chain: &dyn ChainClient,
    refs: &DeploymentRefs<'_>,
    factory: &str,
    clone_addr: &str,
    env_var: &str,
) -> DeploymentLineage {
    // (1) Is the configuration itself one deployment? Only askable when a verification registry
    // is configured; its two pins are immutable, so a definite disagreement is a definite mix.
    let mut unestablished: Option<String> = None;
    if valid_contract_addr(refs.verification_registry_addr) {
        match chain
            .verification_registry_pins(refs.verification_registry_addr)
            .await
        {
            Ok(pins) => {
                if !same_addr(&pins.root_index, factory) {
                    return DeploymentLineage::Foreign(format!(
                        "the configured FACTORY_ADDR {factory} is not the root index the \
                         verification registry {vr} resolves credentials through ({pin}) — the \
                         configuration itself mixes deployments; re-generate the deployment env \
                         from the ledger (scripts/gen-deployment-env.sh) so every ledger-owned \
                         address names one deployment",
                        vr = refs.verification_registry_addr,
                        pin = pins.root_index,
                    ));
                }
                if valid_contract_addr(refs.sbt_addr) && !same_addr(&pins.sbt, refs.sbt_addr) {
                    return DeploymentLineage::Foreign(format!(
                        "the configured SBT_CONSENT_ADDR {sbt} is not the SBT the verification \
                         registry {vr} pins ({pin}) — the configuration itself mixes deployments; \
                         a tag minted there could never satisfy that registry's profileRoot \
                         binding. Re-generate the deployment env from the ledger \
                         (scripts/gen-deployment-env.sh)",
                        sbt = refs.sbt_addr,
                        vr = refs.verification_registry_addr,
                        pin = pins.sbt,
                    ));
                }
            }
            // A failed read is a fact about our connectivity, not about the configuration; it
            // only keeps a clean clone answer from being over-claimed as full membership.
            Err(e) => {
                unestablished = Some(format!(
                    "the verification registry {vr}'s own deployment pins could not be read ({e})",
                    vr = refs.verification_registry_addr,
                ));
            }
        }
    }
    // (2) The clone itself, against the factory's own write-once membership set.
    match chain.factory_is_clone(factory, clone_addr).await {
        Ok(true) => match unestablished {
            None => DeploymentLineage::Member,
            Some(d) => DeploymentLineage::Unknown(format!(
                "{env_var} {clone_addr} is a clone of the configured factory {factory}, but {d}"
            )),
        },
        Ok(false) => DeploymentLineage::Foreign(format!(
            "{env_var} {clone_addr} is not a clone of this deployment's factory {factory} — it \
             belongs to a different (likely superseded) deployment, so a root anchored through it \
             would be unattributable by every verifier on this deployment: integrity and isValid \
             would read fine while the mandatory issuer-whitelist pillar stays permanently \
             unresolved. Deploy or select a clone from this deployment's factory (the Provider \
             page), then update {env_var} — the env sync deliberately preserves per-provider \
             clone addresses across a redeploy, so this variable never moves by itself"
        )),
        Err(e) => DeploymentLineage::Unknown(format!(
            "whether {env_var} {clone_addr} belongs to this deployment's factory {factory} could \
             not be established ({e})"
        )),
    }
}

/// Boot-time sweep over every configured issuer clone, LOGGED rather than gating: the diagnostic
/// must never gate liveness (the `ADMIN_REQUIRE_AUTHORITY` precedent), and each issuance surface
/// re-asks its own clone before acting anyway. Only issuing roles run it — a groomer mounts no
/// issuance routes, so its clone addresses feed no anchor.
pub async fn log_boot_lineage(chain: &dyn ChainClient, cfg: &crate::app::Config) {
    if !cfg.issuance_enabled() {
        return;
    }
    let refs = DeploymentRefs::of(cfg);
    let mut clones: Vec<(String, &str)> = Vec::new();
    if valid_contract_addr(&cfg.profile_issuer_addr) {
        clones.push(("PROFILE_ISSUER_ADDR".to_string(), &cfg.profile_issuer_addr));
    }
    for (record_type, addr) in &cfg.issuer_addrs {
        if valid_contract_addr(addr) {
            clones.push((format!("{record_type}_ISSUER_ADDR"), addr));
        }
    }
    for (env_var, addr) in clones {
        match issuer_clone_lineage(chain, &refs, addr, &env_var).await {
            DeploymentLineage::Member => {
                tracing::info!("deployment lineage: {env_var} {addr} belongs to this deployment");
            }
            DeploymentLineage::Foreign(detail) => {
                tracing::error!(
                    "DEPLOYMENT MISMATCH — issuance through this clone will be refused: {detail}"
                );
            }
            DeploymentLineage::Unknown(detail) => {
                tracing::warn!("deployment lineage unestablished (issuance will warn): {detail}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::MemChain;

    const FACTORY: &str = "0x00000000000000000000000000000000000000fa";
    const VREG: &str = "0x00000000000000000000000000000000000000ff";
    const SBT: &str = "0x00000000000000000000000000000000000000dd";
    const CLONE: &str = "0x00000000000000000000000000000000000000ee";

    fn refs<'a>() -> DeploymentRefs<'a> {
        DeploymentRefs {
            factory_addr: FACTORY,
            verification_registry_addr: VREG,
            sbt_addr: SBT,
        }
    }

    #[tokio::test]
    async fn a_clone_of_the_configured_factory_with_agreeing_pins_is_a_member() {
        let chain = MemChain::new();
        chain.set_registry_pins(VREG, FACTORY, SBT);
        chain.set_factory_clone(FACTORY, CLONE, true);
        let v = issuer_clone_lineage(&chain, &refs(), CLONE, "PROFILE_ISSUER_ADDR").await;
        assert_eq!(v, DeploymentLineage::Member);
    }

    #[tokio::test]
    async fn a_foreign_clone_is_refused_naming_the_clone_the_factory_and_the_fix() {
        let chain = MemChain::new();
        chain.set_registry_pins(VREG, FACTORY, SBT);
        chain.set_factory_clone(FACTORY, CLONE, false);
        match issuer_clone_lineage(&chain, &refs(), CLONE, "PROFILE_ISSUER_ADDR").await {
            DeploymentLineage::Foreign(d) => {
                assert!(d.contains(CLONE), "names the foreign clone: {d}");
                assert!(d.contains(FACTORY), "names the factory asked: {d}");
                assert!(d.contains("PROFILE_ISSUER_ADDR"), "names the env var: {d}");
                assert!(d.contains("preserves per-provider clone addresses"), "names the fix: {d}");
            }
            other => panic!("expected Foreign, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_factory_the_verification_registry_does_not_pin_is_a_mixed_configuration() {
        let chain = MemChain::new();
        chain.set_registry_pins(VREG, "0x00000000000000000000000000000000000000f1", SBT);
        // The clone answer is irrelevant here — a mixed config refuses before it is consulted.
        chain.set_factory_clone(FACTORY, CLONE, true);
        match issuer_clone_lineage(&chain, &refs(), CLONE, "PROFILE_ISSUER_ADDR").await {
            DeploymentLineage::Foreign(d) => {
                assert!(d.contains("FACTORY_ADDR"), "names the mixed variable: {d}");
                assert!(d.contains(VREG), "names the registry whose pin disagrees: {d}");
            }
            other => panic!("expected Foreign, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_sbt_the_verification_registry_does_not_pin_is_a_mixed_configuration() {
        let chain = MemChain::new();
        chain.set_registry_pins(VREG, FACTORY, "0x00000000000000000000000000000000000000d1");
        chain.set_factory_clone(FACTORY, CLONE, true);
        match issuer_clone_lineage(&chain, &refs(), CLONE, "PROFILE_ISSUER_ADDR").await {
            DeploymentLineage::Foreign(d) => {
                assert!(d.contains("SBT_CONSENT_ADDR"), "names the mixed variable: {d}");
            }
            other => panic!("expected Foreign, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unreadable_membership_read_is_unknown_never_a_refusal_or_a_pass() {
        let chain = MemChain::new(); // nothing seeded: both reads fail
        match issuer_clone_lineage(&chain, &refs(), CLONE, "PROFILE_ISSUER_ADDR").await {
            DeploymentLineage::Unknown(d) => {
                assert!(d.contains("could not be established"), "says could-not-check: {d}");
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_member_clone_with_unreadable_registry_pins_is_unknown_not_member() {
        // The clone answer alone must not be over-claimed as full membership: "same deployment as
        // the SBT and registry in use" was not established if the registry never answered.
        let chain = MemChain::new();
        chain.set_factory_clone(FACTORY, CLONE, true);
        match issuer_clone_lineage(&chain, &refs(), CLONE, "PROFILE_ISSUER_ADDR").await {
            DeploymentLineage::Unknown(d) => {
                assert!(d.contains("pins could not be read"), "names the unestablished half: {d}");
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn with_no_verification_registry_configured_the_factory_membership_alone_decides() {
        let chain = MemChain::new();
        chain.set_factory_clone(FACTORY, CLONE, true);
        let r = DeploymentRefs {
            factory_addr: FACTORY,
            verification_registry_addr: "",
            sbt_addr: SBT,
        };
        let v = issuer_clone_lineage(&chain, &r, CLONE, "PROFILE_ISSUER_ADDR").await;
        assert_eq!(v, DeploymentLineage::Member);
    }

    #[tokio::test]
    async fn with_no_factory_configured_the_lineage_is_unknown() {
        let chain = MemChain::new();
        let r = DeploymentRefs {
            factory_addr: "",
            verification_registry_addr: VREG,
            sbt_addr: SBT,
        };
        match issuer_clone_lineage(&chain, &r, CLONE, "PROFILE_ISSUER_ADDR").await {
            DeploymentLineage::Unknown(d) => {
                assert!(d.contains("no FACTORY_ADDR"), "names the missing anchor: {d}");
            }
            other => panic!("expected Unknown, got {other:?}"),
        }
    }
}

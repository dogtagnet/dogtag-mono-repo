//! `GovernanceAction` — the key-holder-agnostic privileged-write abstraction (plan PR-A, Part 2.2).
//!
//! Every privileged on-chain write the control plane performs is gated by ONE of three distinct
//! authorities (plan Part 2): the factory `Ownable2Step` owner (`createIssuer`), the registry
//! `WHITELIST_ADMIN` role (`whitelistFor`/`delistFor`), or the registry `DEFAULT_ADMIN_ROLE`
//! (`adminRevoke`, role grants, verifier/consent-key swaps — behind a 2–3 day timelock). Today a
//! single deployer EOA holds all three; the governance Phase-2 handover is moving `DEFAULT_ADMIN` to
//! the governance signer while the hosted operator key may retain `WHITELIST_ADMIN` + factory
//! ownership.
//!
//! Rather than hardcode "the admin EOA is the authority", we model each write as a value
//! (`GovernanceAction { target, calldata, authority, summary }`) and, at dispatch time, ask the chain
//! WHO holds the required authority:
//!   - the hosted signer holds it  → sign-and-broadcast (the existing `sign_and_send` path), and
//!   - it belongs to someone else  → return the `{target, calldata}` proposal payload for that holder
//!     (e.g. the governance signer / a Safe) to execute out-of-band.
//!
//! This makes the portal survive the Phase-2 split BY CONSTRUCTION: no code path assumes which key
//! holds which role, so an action silently flips from "executed" to "proposed" the moment its role
//! moves off the hosted key.

use serde::Serialize;

use crate::chain::{ChainClient, ChainError};

/// The on-chain authority that gates a `GovernanceAction`.
#[derive(Clone, Debug)]
pub enum Authority {
    /// Ownable / Ownable2Step `owner()` of `owner_target` (the factory's `createIssuer` authority).
    Owner { owner_target: String },
    /// An AccessControl `role` on `role_target`. `default_admin` marks the
    /// `AccessControlDefaultAdminRules` DEFAULT_ADMIN_ROLE, whose current holder is readable via
    /// `defaultAdmin()` (ordinary roles have no single-holder accessor, so their holder stays unknown).
    Role {
        role_target: String,
        role: String,
        default_admin: bool,
    },
}

impl Authority {
    /// A short human label for the proposal payload / audit row.
    pub fn label(&self) -> String {
        match self {
            Authority::Owner { owner_target } => format!("Ownable owner of {owner_target}"),
            Authority::Role {
                role_target,
                role,
                default_admin,
            } => {
                if *default_admin {
                    format!("DEFAULT_ADMIN_ROLE on {role_target}")
                } else {
                    format!("role {role} on {role_target}")
                }
            }
        }
    }
}

/// A privileged control-plane write, described independently of who currently holds its authority.
#[derive(Clone, Debug)]
pub struct GovernanceAction {
    /// The contract the calldata is sent to.
    pub target: String,
    /// ABI-encoded calldata (`0x..`).
    pub calldata: String,
    /// The authority that gates `target`.
    pub authority: Authority,
    /// Human-readable one-line description (audit + proposal payload).
    pub summary: String,
}

/// The outcome of dispatching a `GovernanceAction`.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "disposition", rename_all = "snake_case")]
pub enum Disposition {
    /// The hosted signer held the authority; the action was signed and broadcast.
    Executed {
        #[serde(rename = "txHash")]
        tx_hash: String,
        /// The authority holder (== the hosted signer) for the audit row.
        holder: String,
        summary: String,
    },
    /// The authority belongs to another key (e.g. the governance signer post Phase-2). Nothing was
    /// broadcast; this payload is handed to the holder to execute out-of-band.
    Proposed {
        /// The current holder if resolvable (Owner + DEFAULT_ADMIN), else `null`.
        holder: Option<String>,
        /// The hosted signer that was CHECKED and found not to hold the authority. Without this, a
        /// proposal caused by booting the wrong key looks identical to one caused by the authority
        /// legitimately living on a governance signer.
        #[serde(rename = "hostedSigner")]
        hosted_signer: Option<String>,
        target: String,
        calldata: String,
        authority: String,
        summary: String,
    },
}

impl Disposition {
    /// `true` when this action was actually signed and broadcast.
    pub fn executed(&self) -> bool {
        matches!(self, Disposition::Executed { .. })
    }
}

/// `true` when NOT ONE of `dispositions` reached the chain. Callers surface this so a request that
/// changed nothing on-chain cannot read as a success: the whole set came back as unsigned calldata.
pub fn none_executed(dispositions: &[Disposition]) -> bool {
    !dispositions.is_empty() && !dispositions.iter().any(Disposition::executed)
}

/// Resolve `action`'s authority against the chain and either broadcast (hosted key holds it) or return
/// a proposal payload (someone else holds it). Never assumes which key is the authority — that is the
/// whole point: the same call works before and after the Phase-2 DEFAULT_ADMIN handover.
pub async fn dispatch<C: ChainClient + ?Sized>(
    chain: &C,
    signer_index: u32,
    action: &GovernanceAction,
) -> Result<Disposition, ChainError> {
    let hosted = chain.signer_address(signer_index).await;

    let (holds, holder) = match &action.authority {
        Authority::Owner { owner_target } => {
            let owner = chain.ownable_owner(owner_target).await?;
            let holds = hosted
                .as_deref()
                .map(|h| h.eq_ignore_ascii_case(&owner))
                .unwrap_or(false);
            (holds, Some(owner))
        }
        Authority::Role {
            role_target,
            role,
            default_admin,
        } => {
            let holds = match hosted.as_deref() {
                Some(h) => chain.has_role(role_target, role, h).await?,
                None => false,
            };
            // Only DEFAULT_ADMIN exposes a single-holder accessor; ordinary roles cannot be enumerated.
            let holder = if *default_admin {
                chain.default_admin(role_target).await.ok()
            } else {
                None
            };
            (holds, holder)
        }
    };

    if holds {
        let sent = chain
            .send_action(signer_index, &action.target, &action.calldata)
            .await?;
        Ok(Disposition::Executed {
            tx_hash: sent.tx_hash,
            holder: holder.unwrap_or_else(|| hosted.unwrap_or_default()),
            summary: action.summary.clone(),
        })
    } else {
        Ok(Disposition::Proposed {
            holder,
            hosted_signer: hosted,
            target: action.target.clone(),
            calldata: action.calldata.clone(),
            authority: action.authority.label(),
            summary: action.summary.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::{default_admin_role, whitelist_admin_role, MemChain};

    const FACTORY: &str = "0x00000000000000000000000000000000000000fa";
    const REGISTRY: &str = "0x00000000000000000000000000000000000000a1";
    const HOSTED: &str = "0x00000000000000000000000000000000000000ad";
    const GOV: &str = "0x8e27e11700000000000000000000000000000000";

    fn chain_with_hosted() -> MemChain {
        let c = MemChain::new();
        c.set_signer(0, HOSTED);
        c
    }

    /// Owner action + hosted key IS the factory owner → executed (signed and broadcast).
    #[tokio::test]
    async fn owner_action_executes_when_hosted_key_is_owner() {
        let c = chain_with_hosted();
        c.set_factory_owner(FACTORY, HOSTED);
        let action = GovernanceAction {
            target: FACTORY.into(),
            calldata: "0xabcdef".into(),
            authority: Authority::Owner {
                owner_target: FACTORY.into(),
            },
            summary: "createIssuer".into(),
        };
        let d = dispatch(&c, 0, &action).await.unwrap();
        match d {
            Disposition::Executed { holder, .. } => assert!(holder.eq_ignore_ascii_case(HOSTED)),
            other => panic!("want Executed, got {other:?}"),
        }
    }

    /// Owner action + owner is the governance signer → proposed (nothing broadcast).
    #[tokio::test]
    async fn owner_action_proposes_when_owner_is_governance() {
        let c = chain_with_hosted();
        c.set_factory_owner(FACTORY, GOV);
        let action = GovernanceAction {
            target: FACTORY.into(),
            calldata: "0xabcdef".into(),
            authority: Authority::Owner {
                owner_target: FACTORY.into(),
            },
            summary: "createIssuer".into(),
        };
        match dispatch(&c, 0, &action).await.unwrap() {
            Disposition::Proposed {
                holder, calldata, ..
            } => {
                assert_eq!(
                    holder.as_deref().map(str::to_lowercase),
                    Some(GOV.to_lowercase())
                );
                assert_eq!(calldata, "0xabcdef");
            }
            other => panic!("want Proposed, got {other:?}"),
        }
    }

    /// WHITELIST_ADMIN action + hosted key holds the role → executed.
    #[tokio::test]
    async fn whitelist_admin_executes_when_hosted_holds_role() {
        let c = chain_with_hosted();
        c.set_role(REGISTRY, &whitelist_admin_role(), HOSTED);
        let action = GovernanceAction {
            target: REGISTRY.into(),
            calldata: "0x1234".into(),
            authority: Authority::Role {
                role_target: REGISTRY.into(),
                role: whitelist_admin_role(),
                default_admin: false,
            },
            summary: "whitelistFor".into(),
        };
        assert!(matches!(
            dispatch(&c, 0, &action).await.unwrap(),
            Disposition::Executed { .. }
        ));
    }

    /// DEFAULT_ADMIN action after Phase-2 (role moved to governance) → proposed, with the governance
    /// holder surfaced from `defaultAdmin()`.
    #[tokio::test]
    async fn default_admin_proposes_after_phase2_handover() {
        let c = chain_with_hosted();
        // hosted key does NOT hold DEFAULT_ADMIN; governance does.
        c.set_default_admin(REGISTRY, GOV);
        let action = GovernanceAction {
            target: REGISTRY.into(),
            calldata: "0xdead".into(),
            authority: Authority::Role {
                role_target: REGISTRY.into(),
                role: default_admin_role(),
                default_admin: true,
            },
            summary: "adminRevoke".into(),
        };
        match dispatch(&c, 0, &action).await.unwrap() {
            Disposition::Proposed {
                holder, authority, ..
            } => {
                assert_eq!(
                    holder.as_deref().map(str::to_lowercase),
                    Some(GOV.to_lowercase())
                );
                assert!(authority.contains("DEFAULT_ADMIN"));
            }
            other => panic!("want Proposed, got {other:?}"),
        }
    }
}

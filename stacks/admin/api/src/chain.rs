//! `ChainClient` trait abstracting the ROAX (chainId 135) on-chain surface the CENTRAL/admin backend
//! needs: the `IssuerRegistry` whitelist (`whitelistFor` / `delistFor` / `isWhitelistedFor`) written by
//! the WHITELIST_ADMIN signer, plus issuer-role and governance administration.
//! An Alloy-backed implementation broadcasts real transactions; an in-memory `MemChain` emulates the
//! whitelist and governance surfaces so the full HTTP flow is testable without a live node.
//!
//! Signing (impl §1.8): EIP-1559 with a legacy `gas_price` fallback; chainId pinned to 135.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use alloy::primitives::{Address, Bytes, FixedBytes, B256, U256};
use alloy::sol;
use async_trait::async_trait;

use crate::provider_registry::{IdentityAnchor, ProviderRecord, Standing};

pub const ROAX_CHAIN_ID: u64 = 135;

sol! {
    #[sol(rpc)]
    contract IIssuerRegistry {
        function whitelistFor(bytes32 recordType, address signer) external;
        function delistFor(bytes32 recordType, address signer) external;
        function isWhitelistedFor(bytes32 recordType, address signer) external view returns (bool);
    }

    #[sol(rpc)]
    contract IDogTagSBT {
        // AccessControl surface — the DEFAULT_ADMIN holder grants ISSUER_ROLE to owner-hidden issuers.
        function grantRole(bytes32 role, address account) external;
        function hasRole(bytes32 role, address account) external view returns (bool);
    }

    #[sol(rpc)]
    contract IDogTagIssuerFactory {
        // Ownable2Step onlyOwner: deploy a deterministic EIP-1167 issuer clone. salt = keccak256(recordType, business).
        function createIssuer(string name, bytes32 recordType, address business) external returns (address clone);
        function predictIssuer(bytes32 recordType, address business) external view returns (address);
        function isClone(address) external view returns (bool);
        function rootIssuer(bytes32 root) external view returns (address);
        // Ownable2Step surface — the factory owner is the createIssuer authority (distinct from the registry admin).
        function owner() external view returns (address);
        function pendingOwner() external view returns (address);
    }

    // AccessControlDefaultAdminRules surface on the IssuerRegistry / VerificationRegistry: read the live
    // DEFAULT_ADMIN holder + any pending (timelocked) transfer, and probe arbitrary role membership.
    #[sol(rpc)]
    contract IAccessControlAdmin {
        function hasRole(bytes32 role, address account) external view returns (bool);
        function defaultAdmin() external view returns (address);
        function pendingDefaultAdmin() external view returns (address newAdmin, uint48 acceptSchedule);
    }

    // The generation-2 `ProviderRegistry` REGISTRAR surface (registry plan C-2). Every write here is
    // `onlyOwner` and routes through `GovernanceAction` like every other privileged admin write, so
    // the authority is read live from `owner()` rather than assumed.
    //
    // NOTE the reads are deliberately the ones that answer a REGISTRAR's question. `canCreateService`
    // is NOT among them: its first term is `generationOfFactory[msg.sender]`, so a plain `eth_call`
    // with no `from` answers false for every provider on earth, and even spoofed it folds four terms
    // into one unattributable bool. The raw approval bit comes from the event log below.
    //
    // Deliberately NOT `#[sol(rpc)]`: that generates a contract-instance `provider()` accessor which
    // collides with this contract's OWN `provider(bytes20)` function (E0592). The name is fixed by the
    // deployed ABI - the selector derives from it - so the call cannot be renamed; the reads use the
    // generated `SolCall` types over a raw `eth_call` instead.
    contract IProviderRegistry {
        function registerProvider(
            bytes20 providerId,
            address controller,
            bytes32 publicIdentityDigest,
            uint32 publicIdentitySchema,
            uint16 codec,
            uint8 hashAlgorithm,
            bytes contenthash
        ) external;
        function setProviderStanding(bytes20 providerId, uint8 newStanding) external;
        function setServiceCreationApproval(bytes20 providerId, bytes32 recordType, bool allowed) external;

        function provider(bytes20 providerId) external view returns (Provider memory);
        function publicIdentityAnchor(bytes20 providerId) external view returns (PublicIdentityAnchor memory);
        function providerCount() external view returns (uint256);
        function providerPage(uint256 cursor, uint256 limit)
            external
            view
            returns (bytes20[] memory values, uint256 nextCursor);
        function owner() external view returns (address);

        // The ONLY direct evidence of what a provider is approved for — `_serviceCreationApprovals`
        // is private with no getter. Both leading args are indexed.
        event ServiceCreationApprovalSet(bytes20 indexed providerId, bytes32 indexed recordType, bool allowed);
    }

    // `ProviderRegistry.Provider` (ProviderRegistry.sol:71-80) and `PublicIdentityAnchor` (:84-92).
    // Declared as bare structs so the `sol!` return decoding above matches the deployed ABI exactly;
    // a member out of order decodes silently into the wrong field.
    struct Provider {
        address controller;
        address pendingController;
        bool pendingControllerAccepted;
        bool pendingControllerRequestedByRegistrar;
        address directoryResolver;
        uint64 controllerEpoch;
        uint64 controllerRequestNonce;
        uint8 standing;
    }

    struct PublicIdentityAnchor {
        bytes32 digest;
        uint32 schema;
        uint16 codec;
        uint8 hashAlgorithm;
        bytes contenthash;
        uint64 revision;
        uint64 updatedAtBlock;
    }
}

/// `IssuerRegistry.WHITELIST_ADMIN = keccak256("WHITELIST_ADMIN")` — the role gating whitelistFor/delistFor.
pub fn whitelist_admin_role() -> String {
    use alloy::primitives::keccak256;
    let h: FixedBytes<32> = keccak256(b"WHITELIST_ADMIN");
    format!("0x{}", hex::encode(h.as_slice()))
}

/// `DEFAULT_ADMIN_ROLE = 0x00…00` — the OpenZeppelin AccessControl default admin role (bytes32 zero).
pub fn default_admin_role() -> String {
    format!("0x{}", hex::encode([0u8; 32]))
}

/// `DogTagSBTConsent.ISSUER_ROLE = keccak256("ISSUER")` — gates `mintCustodial`.
pub fn issuer_role_key() -> String {
    use alloy::primitives::keccak256;
    let h: FixedBytes<32> = keccak256(b"ISSUER");
    format!("0x{}", hex::encode(h.as_slice()))
}

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("rpc: {0}")]
    Rpc(String),
    #[error("{0}")]
    Other(String),
}

/// Result of a broadcast: the tx hash.
#[derive(Clone, Debug)]
pub struct SentTx {
    pub tx_hash: String,
}

fn parse_b256(h: &str) -> B256 {
    let s = h.strip_prefix("0x").unwrap_or(h);
    let mut out = [0u8; 32];
    if let Ok(b) = hex::decode(s) {
        if b.len() == 32 {
            out.copy_from_slice(&b);
        }
    }
    B256::from(out)
}

fn parse_addr(h: &str) -> Address {
    h.parse::<Address>().unwrap_or(Address::ZERO)
}

/// Abstract chain surface. Addresses/roots are passed as lowercase `0x..` hex strings.
#[async_trait]
pub trait ChainClient: Send + Sync {
    /// Register the admin signer (32-byte secp256k1 private key) for an account index, with its
    /// derived address. The Alloy impl keeps the key for broadcasting; MemChain keeps only the address.
    async fn register_signer(&self, index: u32, private_key: [u8; 32], address: String);

    /// IssuerRegistry.whitelistFor(recordType, signer) — admin-only write. `record_type` is the
    /// keccak256 bytes32 key (NOT the human label).
    async fn whitelist_for(
        &self,
        account_index: u32,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<SentTx, ChainError>;

    /// IssuerRegistry.delistFor(recordType, signer) — admin-only write.
    async fn delist_for(
        &self,
        account_index: u32,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<SentTx, ChainError>;

    /// IssuerRegistry.isWhitelistedFor(recordType, signer).
    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<bool, ChainError>;

    /// DogTagSBTConsent.grantRole(ISSUER_ROLE, grantee) — broadcast by the admin signer (which holds
    /// DEFAULT_ADMIN_ROLE), granting `grantee` the owner-hidden `mintCustodial` capability.
    async fn grant_issuer_role(
        &self,
        account_index: u32,
        sbt_addr: &str,
        grantee: &str,
    ) -> Result<SentTx, ChainError>;

    /// DogTagSBT.hasRole(ISSUER_ROLE, account) — read so approve can skip an already-granted role.
    async fn has_issuer_role(&self, sbt_addr: &str, account: &str) -> Result<bool, ChainError>;

    // ---- factory / governance surface (PR-A) --------------------------------------------------

    /// The lowercase `0x..` address of the signer registered at `index`, if any. The `GovernanceAction`
    /// dispatcher uses this to decide whether the hosted key HOLDS the required authority (sign-and-send)
    /// or whether it belongs to a governance signer (propose). Alloy derives it from the private key.
    async fn signer_address(&self, index: u32) -> Option<String>;

    /// Broadcast an arbitrary `{target, calldata}` from the signer at `account_index`. The generic
    /// escape hatch the `GovernanceAction` dispatcher and `createIssuer` share (mirrors sign_and_send).
    async fn send_action(
        &self,
        account_index: u32,
        target: &str,
        calldata: &str,
    ) -> Result<SentTx, ChainError>;

    /// DogTagIssuerFactory.predictIssuer(recordType, business) — the deterministic clone address
    /// (`salt = keccak256(recordType, business)`), exact and computable BEFORE any deploy.
    async fn predict_issuer(
        &self,
        factory_addr: &str,
        record_type: &str,
        business: &str,
    ) -> Result<String, ChainError>;

    /// DogTagIssuerFactory.isClone(addr) — was `addr` deployed by this factory.
    async fn is_clone(&self, factory_addr: &str, addr: &str) -> Result<bool, ChainError>;

    /// DogTagIssuerFactory.rootIssuer(root) — the write-once root→clone binding (zero addr if unset).
    async fn root_issuer(&self, factory_addr: &str, root: &str) -> Result<String, ChainError>;

    /// Ownable/Ownable2Step `owner()` on `addr` (e.g. the factory's createIssuer authority).
    async fn ownable_owner(&self, addr: &str) -> Result<String, ChainError>;

    /// Ownable2Step `pendingOwner()` — the queued (un-accepted) owner of a two-step transfer.
    async fn ownable_pending_owner(&self, addr: &str) -> Result<String, ChainError>;

    /// AccessControl `hasRole(role, account)` on `addr` (registry WHITELIST_ADMIN / DEFAULT_ADMIN probe).
    async fn has_role(&self, addr: &str, role: &str, account: &str) -> Result<bool, ChainError>;

    /// AccessControlDefaultAdminRules `defaultAdmin()` — the current DEFAULT_ADMIN holder.
    async fn default_admin(&self, addr: &str) -> Result<String, ChainError>;

    /// AccessControlDefaultAdminRules `pendingDefaultAdmin()` — `(newAdmin, acceptSchedule)`. The
    /// Phase-2 DEFAULT_ADMIN → governance handover surfaces here (newAdmin = governance signer, schedule
    /// = unix ETA). `(zero addr, 0)` when no transfer is pending.
    async fn pending_default_admin(&self, addr: &str) -> Result<(String, u64), ChainError>;

    /// `ProviderRegistry.providerPage(cursor, limit)` → `(providerIds, nextCursor)`.
    /// `_providerIds` is append-only, so the page is stable. `limit` must be in `[1, MAX_PAGE_SIZE]`
    /// and `cursor > length` reverts `BadPage()`.
    async fn provider_page(
        &self,
        registry_addr: &str,
        cursor: u64,
        limit: u64,
    ) -> Result<(Vec<String>, u64), ChainError>;

    /// `ProviderRegistry.provider(providerId)`. Does NOT revert for an unknown id — it answers a
    /// zero-filled struct — so `ProviderRecord::registered` (`controller != 0`) is the existence
    /// test, never the standing.
    async fn provider_record(
        &self,
        registry_addr: &str,
        provider_id: &str,
    ) -> Result<ProviderRecord, ChainError>;

    /// `ProviderRegistry.publicIdentityAnchor(providerId)` — the registrar's own identity assertion.
    async fn provider_identity_anchor(
        &self,
        registry_addr: &str,
        provider_id: &str,
    ) -> Result<IdentityAnchor, ChainError>;

    /// The `ServiceCreationApprovalSet` log for one provider, in ascending `(block, logIndex)` order,
    /// as `(recordTypeKey, allowed)` pairs. The raw approval mapping is private with no getter, so
    /// this log is the only direct evidence of what a provider is approved for; a read that FAILS
    /// must surface as an error and never as an empty approval set.
    async fn service_creation_approval_log(
        &self,
        registry_addr: &str,
        provider_id: &str,
    ) -> Result<Vec<(String, bool)>, ChainError>;
}

// --------------------------------------------------------------------------------------------
// MemChain — in-memory emulation of the whitelist + governance surfaces.
// --------------------------------------------------------------------------------------------

#[derive(Default)]
struct MemChainInner {
    /// (registry_addr, record_type, signer) -> whitelisted.
    whitelist: HashMap<(String, String, String), bool>,
    /// (sbt_addr, account) holding DogTagSBT.ISSUER_ROLE.
    issuer_roles: std::collections::HashSet<(String, String)>,
    /// admin signer addresses by account index.
    signers: HashMap<u32, String>,
    /// factory_addr -> Ownable owner.
    factory_owner: HashMap<String, String>,
    /// factory_addr -> Ownable2Step pending owner.
    factory_pending_owner: HashMap<String, String>,
    /// (target_addr, role, account) holding an AccessControl role.
    roles: std::collections::HashSet<(String, String, String)>,
    /// target_addr -> AccessControlDefaultAdminRules current DEFAULT_ADMIN.
    default_admin: HashMap<String, String>,
    /// target_addr -> (pending new DEFAULT_ADMIN, unix acceptSchedule).
    pending_default_admin: HashMap<String, (String, u64)>,
    /// (factory_addr, clone_addr) deployed by the factory.
    clones: std::collections::HashSet<(String, String)>,
    /// (factory_addr, root) -> issuing clone (write-once).
    root_issuer: HashMap<(String, String), String>,
    /// (provider_registry_addr, provider_id) -> the registered provider record.
    providers: HashMap<(String, String), ProviderRecord>,
    /// Registration order per registry, mirroring the contract's append-only `_providerIds`.
    provider_ids: HashMap<String, Vec<String>>,
    /// (provider_registry_addr, provider_id) -> identity anchor.
    identity_anchors: HashMap<(String, String), IdentityAnchor>,
    /// (provider_registry_addr, provider_id) -> the ordered `ServiceCreationApprovalSet` log. A log
    /// rather than a map, because the fold that turns it into current state is what production reads.
    approval_log: HashMap<(String, String), Vec<(String, bool)>>,
    /// Registries whose reads are forced to fail, so a route's could-not-run arm is reachable. A fake
    /// that cannot fail cannot exercise the state that exists for failure.
    failing_reads: std::collections::HashSet<String>,
    nonce: u64,
}

#[derive(Clone, Default)]
pub struct MemChain {
    inner: Arc<Mutex<MemChainInner>>,
}

impl MemChain {
    pub fn new() -> Self {
        Self::default()
    }
    /// Register an admin signer address for an account index (test harness wires this from custody).
    pub fn set_signer(&self, index: u32, address: &str) {
        self.inner
            .lock()
            .unwrap()
            .signers
            .insert(index, address.to_lowercase());
    }
    fn next_tx(g: &mut MemChainInner) -> String {
        g.nonce += 1;
        format!("0x{:064x}", g.nonce)
    }

    /// Seed the Ownable owner of a factory (test harness).
    pub fn set_factory_owner(&self, factory_addr: &str, owner: &str) {
        self.inner
            .lock()
            .unwrap()
            .factory_owner
            .insert(factory_addr.to_lowercase(), owner.to_lowercase());
    }
    /// Seed a pending Ownable2Step owner transfer (test harness).
    pub fn set_factory_pending_owner(&self, factory_addr: &str, pending: &str) {
        self.inner
            .lock()
            .unwrap()
            .factory_pending_owner
            .insert(factory_addr.to_lowercase(), pending.to_lowercase());
    }
    /// Grant an AccessControl role to `account` on `target` (test harness).
    pub fn set_role(&self, target: &str, role: &str, account: &str) {
        self.inner.lock().unwrap().roles.insert((
            target.to_lowercase(),
            role.to_lowercase(),
            account.to_lowercase(),
        ));
    }
    /// Seed the current DEFAULT_ADMIN of a registry (test harness).
    pub fn set_default_admin(&self, target: &str, admin: &str) {
        self.inner
            .lock()
            .unwrap()
            .default_admin
            .insert(target.to_lowercase(), admin.to_lowercase());
    }
    /// Seed a pending DEFAULT_ADMIN transfer (the Phase-2 handover shape) (test harness).
    pub fn set_pending_default_admin(&self, target: &str, new_admin: &str, schedule: u64) {
        self.inner
            .lock()
            .unwrap()
            .pending_default_admin
            .insert(target.to_lowercase(), (new_admin.to_lowercase(), schedule));
    }

    /// Force every `ProviderRegistry` read against `registry_addr` to fail, so a route's
    /// could-not-run arm is reachable in a test. Default off; a store that cannot fail cannot
    /// exercise the one state the three-way split exists for.
    pub fn set_failing_provider_reads(&self, registry_addr: &str, failing: bool) {
        let mut g = self.inner.lock().unwrap();
        if failing {
            g.failing_reads.insert(registry_addr.to_lowercase());
        } else {
            g.failing_reads.remove(&registry_addr.to_lowercase());
        }
    }

    fn provider_reads_fail(g: &MemChainInner, registry_addr: &str) -> Result<(), ChainError> {
        if g.failing_reads.contains(&registry_addr.to_lowercase()) {
            return Err(ChainError::Rpc("provider read failed (seeded)".into()));
        }
        Ok(())
    }
}

/// Deterministic clone-address preview for MemChain: last 20 bytes of `keccak256(recordType ++
/// business ++ factory)`. NOT the real CREATE2 address (AlloyChain reads the exact on-chain
/// `predictIssuer`), but stable across predict/create so the in-memory flow is coherent and testable.
/// Apply a `ProviderRegistry` registrar write to `MemChain` state, modelling the contract's own
/// guards. Anything that is not one of the three registrar selectors is ignored, so this is a no-op
/// for every other `send_action` caller (whitelist grants, factory deploys).
///
/// The reverts are emulated because they are exactly what a route must not provoke: the contract
/// refuses a redundant `setServiceCreationApproval` with `NoChange()`, and a fake that silently
/// accepted it would let a route ship that sends a pointless transaction and reports it as success.
fn apply_provider_registry_calldata(
    g: &mut MemChainInner,
    target: &str,
    calldata: &str,
) -> Result<(), ChainError> {
    use alloy::sol_types::SolCall;

    let Ok(data) = hex::decode(calldata.strip_prefix("0x").unwrap_or(calldata)) else {
        return Ok(());
    };
    if data.len() < 4 {
        return Ok(());
    }
    let registry = target.to_lowercase();

    if let Ok(c) = IProviderRegistry::registerProviderCall::abi_decode(&data, true) {
        let id = format!("0x{}", hex::encode(c.providerId.as_slice()));
        if c.providerId.is_zero() {
            return Err(ChainError::Other("tx reverted: ZeroProviderId".into()));
        }
        if c.controller.is_zero() {
            return Err(ChainError::Other("tx reverted: ZeroAddress".into()));
        }
        if g.providers.contains_key(&(registry.clone(), id.clone())) {
            return Err(ChainError::Other("tx reverted: AlreadyRegistered".into()));
        }
        if c.publicIdentityDigest.is_zero() || c.publicIdentitySchema == 0 || c.hashAlgorithm == 0 {
            return Err(ChainError::Other("tx reverted: BadIdentityAnchor".into()));
        }
        g.providers.insert(
            (registry.clone(), id.clone()),
            ProviderRecord {
                provider_id: id.clone(),
                controller: format!("{:#x}", c.controller),
                pending_controller: zero_addr(),
                controller_epoch: 1,
                // The contract registers at PENDING, never ACTIVE. Modelling this is what makes the
                // missing standing step visible in a test rather than only on a live chain.
                standing: Standing::Pending,
                registered: true,
            },
        );
        g.provider_ids.entry(registry.clone()).or_default().push(id.clone());
        g.identity_anchors.insert(
            (registry, id),
            IdentityAnchor {
                digest: format!("0x{}", hex::encode(c.publicIdentityDigest.as_slice())),
                schema: c.publicIdentitySchema,
                codec: c.codec,
                hash_algorithm: c.hashAlgorithm,
                revision: 1,
                updated_at_block: 1,
            },
        );
        return Ok(());
    }

    if let Ok(c) = IProviderRegistry::setProviderStandingCall::abi_decode(&data, true) {
        let id = format!("0x{}", hex::encode(c.providerId.as_slice()));
        let rec = g
            .providers
            .get_mut(&(registry, id))
            .ok_or_else(|| ChainError::Other("tx reverted: UnknownProvider".into()))?;
        let next = Standing::from_u8(c.newStanding);
        if !next.is_settable() {
            return Err(ChainError::Other("tx reverted: InvalidStanding".into()));
        }
        if rec.standing == Standing::Retired {
            return Err(ChainError::Other("tx reverted: RetiredStanding".into()));
        }
        if rec.standing == next {
            return Err(ChainError::Other("tx reverted: NoChange".into()));
        }
        rec.standing = next;
        return Ok(());
    }

    if let Ok(c) = IProviderRegistry::setServiceCreationApprovalCall::abi_decode(&data, true) {
        let id = format!("0x{}", hex::encode(c.providerId.as_slice()));
        if !g.providers.contains_key(&(registry.clone(), id.clone())) {
            return Err(ChainError::Other("tx reverted: UnknownProvider".into()));
        }
        if c.recordType.is_zero() {
            return Err(ChainError::Other("tx reverted: InvalidServiceMetadata".into()));
        }
        let key = format!("0x{}", hex::encode(c.recordType.as_slice()));
        let log = g.approval_log.entry((registry, id)).or_default();
        let current = log
            .iter()
            .rev()
            .find(|(k, _)| k.eq_ignore_ascii_case(&key))
            .map(|(_, allowed)| *allowed)
            .unwrap_or(false);
        if current == c.allowed {
            return Err(ChainError::Other("tx reverted: NoChange".into()));
        }
        log.push((key, c.allowed));
        return Ok(());
    }

    Ok(())
}

fn mem_predict_clone(factory_addr: &str, record_type: &str, business: &str) -> String {
    use alloy::primitives::keccak256;
    let mut buf = Vec::new();
    buf.extend_from_slice(parse_b256(record_type).as_slice());
    buf.extend_from_slice(parse_addr(business).as_slice());
    buf.extend_from_slice(parse_addr(factory_addr).as_slice());
    let h = keccak256(&buf);
    format!("0x{}", hex::encode(&h.as_slice()[12..32]))
}

#[async_trait]
impl ChainClient for MemChain {
    async fn register_signer(&self, index: u32, _private_key: [u8; 32], address: String) {
        self.inner
            .lock()
            .unwrap()
            .signers
            .insert(index, address.to_lowercase());
    }

    async fn whitelist_for(
        &self,
        account_index: u32,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<SentTx, ChainError> {
        let mut g = self.inner.lock().unwrap();
        // emulate onlyRole(WHITELIST_ADMIN): require a registered admin signer at this index.
        g.signers
            .get(&account_index)
            .cloned()
            .ok_or_else(|| ChainError::Other("no admin signer for index".into()))?;
        g.whitelist.insert(
            (
                registry_addr.to_lowercase(),
                record_type.to_lowercase(),
                signer.to_lowercase(),
            ),
            true,
        );
        let tx_hash = Self::next_tx(&mut g);
        Ok(SentTx { tx_hash })
    }

    async fn delist_for(
        &self,
        account_index: u32,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<SentTx, ChainError> {
        let mut g = self.inner.lock().unwrap();
        g.signers
            .get(&account_index)
            .cloned()
            .ok_or_else(|| ChainError::Other("no admin signer for index".into()))?;
        g.whitelist.insert(
            (
                registry_addr.to_lowercase(),
                record_type.to_lowercase(),
                signer.to_lowercase(),
            ),
            false,
        );
        let tx_hash = Self::next_tx(&mut g);
        Ok(SentTx { tx_hash })
    }

    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<bool, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.whitelist
            .get(&(
                registry_addr.to_lowercase(),
                record_type.to_lowercase(),
                signer.to_lowercase(),
            ))
            .copied()
            .unwrap_or(false))
    }

    async fn grant_issuer_role(
        &self,
        account_index: u32,
        sbt_addr: &str,
        grantee: &str,
    ) -> Result<SentTx, ChainError> {
        let mut g = self.inner.lock().unwrap();
        // emulate onlyRole(DEFAULT_ADMIN_ROLE): require a registered admin signer at this index.
        g.signers
            .get(&account_index)
            .cloned()
            .ok_or_else(|| ChainError::Other("no admin signer for index".into()))?;
        g.issuer_roles
            .insert((sbt_addr.to_lowercase(), grantee.to_lowercase()));
        let tx_hash = Self::next_tx(&mut g);
        Ok(SentTx { tx_hash })
    }

    async fn has_issuer_role(&self, sbt_addr: &str, account: &str) -> Result<bool, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.issuer_roles
            .contains(&(sbt_addr.to_lowercase(), account.to_lowercase())))
    }

    async fn signer_address(&self, index: u32) -> Option<String> {
        self.inner.lock().unwrap().signers.get(&index).cloned()
    }

    async fn send_action(
        &self,
        account_index: u32,
        target: &str,
        calldata: &str,
    ) -> Result<SentTx, ChainError> {
        let mut g = self.inner.lock().unwrap();
        g.signers
            .get(&account_index)
            .cloned()
            .ok_or_else(|| ChainError::Other("no signer for index".into()))?;
        // A ProviderRegistry registrar write is APPLIED rather than merely counted, so a test can walk
        // register -> standing -> approve and read the result back through the same code path
        // production reads. Its guards (`AlreadyRegistered`, `NoChange`, `RetiredStanding`) are
        // modelled too - a fake that accepts every write cannot catch a route that sends a redundant
        // one, and `setServiceCreationApproval` really does revert on a no-op toggle.
        apply_provider_registry_calldata(&mut g, target, calldata)?;
        let tx_hash = Self::next_tx(&mut g);
        Ok(SentTx { tx_hash })
    }

    async fn predict_issuer(
        &self,
        factory_addr: &str,
        record_type: &str,
        business: &str,
    ) -> Result<String, ChainError> {
        Ok(mem_predict_clone(factory_addr, record_type, business))
    }

    async fn provider_page(
        &self,
        registry_addr: &str,
        cursor: u64,
        limit: u64,
    ) -> Result<(Vec<String>, u64), ChainError> {
        let g = self.inner.lock().unwrap();
        Self::provider_reads_fail(&g, registry_addr)?;
        let ids = g
            .provider_ids
            .get(&registry_addr.to_lowercase())
            .cloned()
            .unwrap_or_default();
        let len = ids.len() as u64;
        // Mirrors `_checkPage` (ProviderRegistry.sol:1156): `cursor == length` is legal and returns
        // an empty page, which is the caller's terminate condition.
        if limit == 0 || limit > 100 || cursor > len {
            return Err(ChainError::Other("tx reverted: BadPage".into()));
        }
        let end = (cursor + limit).min(len);
        Ok((ids[cursor as usize..end as usize].to_vec(), end))
    }

    async fn provider_record(
        &self,
        registry_addr: &str,
        provider_id: &str,
    ) -> Result<ProviderRecord, ChainError> {
        let g = self.inner.lock().unwrap();
        Self::provider_reads_fail(&g, registry_addr)?;
        Ok(g.providers
            .get(&(registry_addr.to_lowercase(), provider_id.to_lowercase()))
            .cloned()
            // An unknown id reads back zero-filled rather than reverting, exactly as the contract
            // does — `registered: false` is the existence answer, never the standing.
            .unwrap_or_else(|| ProviderRecord {
                provider_id: provider_id.to_lowercase(),
                controller: zero_addr(),
                pending_controller: zero_addr(),
                controller_epoch: 0,
                standing: Standing::None,
                registered: false,
            }))
    }

    async fn provider_identity_anchor(
        &self,
        registry_addr: &str,
        provider_id: &str,
    ) -> Result<IdentityAnchor, ChainError> {
        let g = self.inner.lock().unwrap();
        Self::provider_reads_fail(&g, registry_addr)?;
        Ok(g.identity_anchors
            .get(&(registry_addr.to_lowercase(), provider_id.to_lowercase()))
            .cloned()
            .unwrap_or_else(|| IdentityAnchor {
                digest: format!("0x{}", hex::encode([0u8; 32])),
                schema: 0,
                codec: 0,
                hash_algorithm: 0,
                revision: 0,
                updated_at_block: 0,
            }))
    }

    async fn service_creation_approval_log(
        &self,
        registry_addr: &str,
        provider_id: &str,
    ) -> Result<Vec<(String, bool)>, ChainError> {
        let g = self.inner.lock().unwrap();
        Self::provider_reads_fail(&g, registry_addr)?;
        Ok(g.approval_log
            .get(&(registry_addr.to_lowercase(), provider_id.to_lowercase()))
            .cloned()
            .unwrap_or_default())
    }

    async fn is_clone(&self, factory_addr: &str, addr: &str) -> Result<bool, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.clones
            .contains(&(factory_addr.to_lowercase(), addr.to_lowercase())))
    }

    async fn root_issuer(&self, factory_addr: &str, root: &str) -> Result<String, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.root_issuer
            .get(&(factory_addr.to_lowercase(), root.to_lowercase()))
            .cloned()
            .unwrap_or_else(zero_addr))
    }

    async fn ownable_owner(&self, addr: &str) -> Result<String, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.factory_owner
            .get(&addr.to_lowercase())
            .cloned()
            .unwrap_or_else(zero_addr))
    }

    async fn ownable_pending_owner(&self, addr: &str) -> Result<String, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.factory_pending_owner
            .get(&addr.to_lowercase())
            .cloned()
            .unwrap_or_else(zero_addr))
    }

    async fn has_role(&self, addr: &str, role: &str, account: &str) -> Result<bool, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.roles.contains(&(
            addr.to_lowercase(),
            role.to_lowercase(),
            account.to_lowercase(),
        )))
    }

    async fn default_admin(&self, addr: &str) -> Result<String, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.default_admin
            .get(&addr.to_lowercase())
            .cloned()
            .unwrap_or_else(zero_addr))
    }

    async fn pending_default_admin(&self, addr: &str) -> Result<(String, u64), ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.pending_default_admin
            .get(&addr.to_lowercase())
            .cloned()
            .unwrap_or_else(|| (zero_addr(), 0)))
    }
}

/// The canonical all-zero address as lowercase `0x..` (unset owner / admin / root sentinel).
fn zero_addr() -> String {
    format!("0x{}", hex::encode([0u8; 20]))
}

// --------------------------------------------------------------------------------------------
// Calldata encoders (canonical typed ABI).
// --------------------------------------------------------------------------------------------

pub fn whitelist_for_calldata(record_type: &str, signer: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IIssuerRegistry::whitelistForCall {
        recordType: parse_b256(record_type),
        signer: parse_addr(signer),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

pub fn delist_for_calldata(record_type: &str, signer: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IIssuerRegistry::delistForCall {
        recordType: parse_b256(record_type),
        signer: parse_addr(signer),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

pub fn grant_issuer_role_calldata(grantee: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IDogTagSBT::grantRoleCall {
        role: parse_b256(&issuer_role_key()),
        account: parse_addr(grantee),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

/// ABI-encoded `DogTagIssuerFactory.createIssuer(name, recordType, business)` calldata. The bytes32
/// `record_type` is the salt key (canonically `keccak256(recordType label)` — see `record_type_key`).
pub fn create_issuer_calldata(name: &str, record_type: &str, business: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IDogTagIssuerFactory::createIssuerCall {
        name: name.to_string(),
        recordType: parse_b256(record_type),
        business: parse_addr(business),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

/// `ProviderRegistry.registerProvider(...)` calldata. Every argument is the registrar's own
/// assertion; the contract validates only that `providerId` and `controller` are non-zero and that
/// the anchor's digest/schema/hashAlgorithm are non-zero (`BadIdentityAnchor()`).
pub fn register_provider_calldata(
    provider_id: &str,
    controller: &str,
    identity_digest: &str,
    schema: u32,
    codec: u16,
    hash_algorithm: u8,
) -> String {
    use alloy::sol_types::SolCall;
    let call = IProviderRegistry::registerProviderCall {
        providerId: parse_b160(provider_id),
        controller: parse_addr(controller),
        publicIdentityDigest: parse_b256(identity_digest),
        publicIdentitySchema: schema,
        codec,
        hashAlgorithm: hash_algorithm,
        // The registrar publishes no content for the identity statement, so there is nothing to
        // address. An identifier for bytes nobody serves is a claim that cannot be checked.
        contenthash: Bytes::new(),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

/// `ProviderRegistry.setProviderStanding(providerId, newStanding)` calldata.
pub fn set_provider_standing_calldata(provider_id: &str, standing: Standing) -> String {
    use alloy::sol_types::SolCall;
    let call = IProviderRegistry::setProviderStandingCall {
        providerId: parse_b160(provider_id),
        newStanding: standing.as_u8(),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

/// `ProviderRegistry.setServiceCreationApproval(providerId, recordType, allowed)` calldata.
pub fn set_service_creation_approval_calldata(
    provider_id: &str,
    record_type_key: &str,
    allowed: bool,
) -> String {
    use alloy::sol_types::SolCall;
    let call = IProviderRegistry::setServiceCreationApprovalCall {
        providerId: parse_b160(provider_id),
        recordType: parse_b256(record_type_key),
        allowed,
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

/// Parse a `bytes20` provider id. A malformed value yields zero, which every registrar write then
/// refuses on-chain with `ZeroProviderId()` — but the routes validate the shape first, so a
/// malformed id is a 400 rather than a wasted transaction.
fn parse_b160(h: &str) -> FixedBytes<20> {
    let s = h.strip_prefix("0x").or_else(|| h.strip_prefix("0X")).unwrap_or(h);
    let mut out = [0u8; 20];
    if let Ok(b) = hex::decode(s) {
        if b.len() == 20 {
            out.copy_from_slice(&b);
        }
    }
    FixedBytes::<20>::from(out)
}

// --------------------------------------------------------------------------------------------
// AlloyChain — real ROAX/anvil-backed client using a derived signer set.
// --------------------------------------------------------------------------------------------

pub struct AlloyChain {
    pub rpc_url: String,
    /// EIP-155 chain id used when signing legacy txs (default `ROAX_CHAIN_ID`; overridable via `CHAIN_ID`).
    pub chain_id: u64,
    /// account index -> alloy local signer (registered at unlock time).
    signers: Mutex<HashMap<u32, alloy::signers::local::PrivateKeySigner>>,
}

impl AlloyChain {
    pub fn new(rpc_url: String) -> Self {
        AlloyChain {
            rpc_url,
            chain_id: ROAX_CHAIN_ID,
            signers: Mutex::new(HashMap::new()),
        }
    }
    /// Override the EIP-155 chain id (config-only chain swap; default stays `ROAX_CHAIN_ID` = 135).
    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = chain_id;
        self
    }
    fn signer(&self, index: u32) -> Option<alloy::signers::local::PrivateKeySigner> {
        self.signers.lock().unwrap().get(&index).cloned()
    }

    /// A plain `eth_call` with NO `from`, used for the `ProviderRegistry` reads whose generated
    /// contract instance cannot be constructed (its `provider(bytes20)` collides with alloy's own
    /// instance accessor).
    ///
    /// Sending no `from` is correct for every read this is used for — they are all pure functions of
    /// their arguments. It would be WRONG for `canCreateService`, whose first term is
    /// `generationOfFactory[msg.sender]` and which therefore answers false for everyone when asked
    /// with no caller; that read is deliberately not made here.
    async fn raw_call(&self, to: &str, input: &[u8]) -> Result<Bytes, ChainError> {
        use alloy::network::TransactionBuilder;
        use alloy::providers::{Provider, ProviderBuilder};
        use alloy::rpc::types::TransactionRequest;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let tx = TransactionRequest::default()
            .with_to(parse_addr(to))
            .with_input(Bytes::from(input.to_vec()));
        provider
            .call(&tx)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))
    }

    /// Sign+broadcast a tx FROM the signer at `account_index` to `to` with `calldata`. EIP-1559 with
    /// a legacy gas_price fallback; chainId pinned to 135.
    async fn sign_and_send(
        &self,
        account_index: u32,
        to: &str,
        calldata: &str,
    ) -> Result<SentTx, ChainError> {
        use alloy::network::EthereumWallet;
        use alloy::network::TransactionBuilder;
        use alloy::providers::{Provider, ProviderBuilder};
        use alloy::rpc::types::TransactionRequest;

        let signer = self
            .signer(account_index)
            .ok_or_else(|| ChainError::Other("no signer for index (unlocked?)".into()))?;
        let wallet = EthereumWallet::from(signer);
        let provider = ProviderBuilder::new()
            .with_recommended_fillers()
            .wallet(wallet)
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        let data = Bytes::from(
            hex::decode(calldata.strip_prefix("0x").unwrap_or(calldata))
                .map_err(|e| ChainError::Other(format!("bad calldata: {e}")))?,
        );
        // LEGACY pricing on ROAX: the node's base fee is ~7 wei but its mempool only mines txs at the
        // ~1 gwei eth_gasPrice. Alloy's EIP-1559 filler derives maxFeePerGas from the (tiny) base fee,
        // producing an underpriced tx that the node ACCEPTS but never mines (stuck forever). Read
        // eth_gasPrice and send a legacy tx (mirrors the working `cast send --legacy`).
        let gp = provider
            .get_gas_price()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let tx = TransactionRequest::default()
            .with_to(parse_addr(to))
            .with_input(data)
            .with_value(U256::ZERO)
            .with_chain_id(self.chain_id)
            .with_gas_price(gp);

        let pending = provider
            .send_transaction(tx)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        // Wait for the tx to be mined so subsequent on-chain reads reflect it.
        let receipt = pending
            .get_receipt()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        if !receipt.status() {
            return Err(ChainError::Other("tx reverted".into()));
        }
        let tx_hash = format!("{:#x}", receipt.transaction_hash);
        Ok(SentTx { tx_hash })
    }
}

#[async_trait]
impl ChainClient for AlloyChain {
    async fn register_signer(&self, index: u32, private_key: [u8; 32], _address: String) {
        if let Ok(s) = alloy::signers::local::PrivateKeySigner::from_bytes(&B256::from(private_key))
        {
            self.signers.lock().unwrap().insert(index, s);
        }
    }

    async fn whitelist_for(
        &self,
        account_index: u32,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<SentTx, ChainError> {
        let calldata = whitelist_for_calldata(record_type, signer);
        self.sign_and_send(account_index, registry_addr, &calldata)
            .await
    }

    async fn delist_for(
        &self,
        account_index: u32,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<SentTx, ChainError> {
        let calldata = delist_for_calldata(record_type, signer);
        self.sign_and_send(account_index, registry_addr, &calldata)
            .await
    }

    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<bool, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IIssuerRegistry::new(parse_addr(registry_addr), provider);
        let r = c
            .isWhitelistedFor(parse_b256(record_type), parse_addr(signer))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }

    async fn grant_issuer_role(
        &self,
        account_index: u32,
        sbt_addr: &str,
        grantee: &str,
    ) -> Result<SentTx, ChainError> {
        let calldata = grant_issuer_role_calldata(grantee);
        self.sign_and_send(account_index, sbt_addr, &calldata).await
    }

    async fn has_issuer_role(&self, sbt_addr: &str, account: &str) -> Result<bool, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagSBT::new(parse_addr(sbt_addr), provider);
        let r = c
            .hasRole(parse_b256(&issuer_role_key()), parse_addr(account))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }

    async fn signer_address(&self, index: u32) -> Option<String> {
        self.signer(index).map(|s| format!("{:#x}", s.address()))
    }

    async fn send_action(
        &self,
        account_index: u32,
        target: &str,
        calldata: &str,
    ) -> Result<SentTx, ChainError> {
        self.sign_and_send(account_index, target, calldata).await
    }

    async fn predict_issuer(
        &self,
        factory_addr: &str,
        record_type: &str,
        business: &str,
    ) -> Result<String, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagIssuerFactory::new(parse_addr(factory_addr), provider);
        let r = c
            .predictIssuer(parse_b256(record_type), parse_addr(business))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(format!("{:#x}", r._0))
    }

    async fn is_clone(&self, factory_addr: &str, addr: &str) -> Result<bool, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagIssuerFactory::new(parse_addr(factory_addr), provider);
        let r = c
            .isClone(parse_addr(addr))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }

    async fn root_issuer(&self, factory_addr: &str, root: &str) -> Result<String, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagIssuerFactory::new(parse_addr(factory_addr), provider);
        let r = c
            .rootIssuer(parse_b256(root))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(format!("{:#x}", r._0))
    }

    async fn ownable_owner(&self, addr: &str) -> Result<String, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagIssuerFactory::new(parse_addr(addr), provider);
        let r = c
            .owner()
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(format!("{:#x}", r._0))
    }

    async fn ownable_pending_owner(&self, addr: &str) -> Result<String, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagIssuerFactory::new(parse_addr(addr), provider);
        let r = c
            .pendingOwner()
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(format!("{:#x}", r._0))
    }

    async fn has_role(&self, addr: &str, role: &str, account: &str) -> Result<bool, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IAccessControlAdmin::new(parse_addr(addr), provider);
        let r = c
            .hasRole(parse_b256(role), parse_addr(account))
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(r._0)
    }

    async fn default_admin(&self, addr: &str) -> Result<String, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IAccessControlAdmin::new(parse_addr(addr), provider);
        let r = c
            .defaultAdmin()
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok(format!("{:#x}", r._0))
    }

    async fn pending_default_admin(&self, addr: &str) -> Result<(String, u64), ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IAccessControlAdmin::new(parse_addr(addr), provider);
        let r = c
            .pendingDefaultAdmin()
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        Ok((format!("{:#x}", r.newAdmin), r.acceptSchedule.to::<u64>()))
    }

    async fn provider_page(
        &self,
        registry_addr: &str,
        cursor: u64,
        limit: u64,
    ) -> Result<(Vec<String>, u64), ChainError> {
        use alloy::sol_types::SolCall;
        let call = IProviderRegistry::providerPageCall {
            cursor: U256::from(cursor),
            limit: U256::from(limit),
        };
        let out = self.raw_call(registry_addr, &call.abi_encode()).await?;
        let r = IProviderRegistry::providerPageCall::abi_decode_returns(&out, true)
            .map_err(|e| ChainError::Other(format!("providerPage decode: {e}")))?;
        Ok((
            r.values
                .iter()
                .map(|v| format!("0x{}", hex::encode(v.as_slice())))
                .collect(),
            r.nextCursor.to::<u64>(),
        ))
    }

    async fn provider_record(
        &self,
        registry_addr: &str,
        provider_id: &str,
    ) -> Result<ProviderRecord, ChainError> {
        use alloy::sol_types::SolCall;
        let call = IProviderRegistry::providerCall {
            providerId: parse_b160(provider_id),
        };
        let out = self.raw_call(registry_addr, &call.abi_encode()).await?;
        let p = IProviderRegistry::providerCall::abi_decode_returns(&out, true)
            .map_err(|e| ChainError::Other(format!("provider decode: {e}")))?
            ._0;
        Ok(ProviderRecord {
            provider_id: provider_id.to_lowercase(),
            controller: format!("{:#x}", p.controller),
            pending_controller: format!("{:#x}", p.pendingController),
            controller_epoch: p.controllerEpoch,
            standing: Standing::from_u8(p.standing),
            // `provider()` does not revert for an unknown id, so a zero controller — the contract's
            // own existence sentinel (`_requireProvider`) — is what "not registered" means here.
            registered: !p.controller.is_zero(),
        })
    }

    async fn provider_identity_anchor(
        &self,
        registry_addr: &str,
        provider_id: &str,
    ) -> Result<IdentityAnchor, ChainError> {
        use alloy::sol_types::SolCall;
        let call = IProviderRegistry::publicIdentityAnchorCall {
            providerId: parse_b160(provider_id),
        };
        let out = self.raw_call(registry_addr, &call.abi_encode()).await?;
        let a = IProviderRegistry::publicIdentityAnchorCall::abi_decode_returns(&out, true)
            .map_err(|e| ChainError::Other(format!("publicIdentityAnchor decode: {e}")))?
            ._0;
        Ok(IdentityAnchor {
            digest: format!("0x{}", hex::encode(a.digest.as_slice())),
            schema: a.schema,
            codec: a.codec,
            hash_algorithm: a.hashAlgorithm,
            revision: a.revision,
            updated_at_block: a.updatedAtBlock,
        })
    }

    async fn service_creation_approval_log(
        &self,
        registry_addr: &str,
        provider_id: &str,
    ) -> Result<Vec<(String, bool)>, ChainError> {
        use alloy::providers::{Provider, ProviderBuilder};
        use alloy::rpc::types::Filter;
        use alloy::sol_types::{SolEvent, SolValue};

        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        // topic1 is the indexed `bytes20 providerId`. Solidity left-aligns a short fixed-bytes value
        // in its topic word, so the filter word is the id followed by 12 zero bytes — right-aligning
        // it (the address convention) matches no log at all, which would read exactly like "this
        // provider has never been approved for anything".
        let mut topic1 = [0u8; 32];
        topic1[..20].copy_from_slice(parse_b160(provider_id).as_slice());
        let filter = Filter::new()
            .address(parse_addr(registry_addr))
            .event_signature(IProviderRegistry::ServiceCreationApprovalSet::SIGNATURE_HASH)
            .topic1(B256::from(topic1))
            .from_block(0u64);
        let logs = provider
            .get_logs(&filter)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        let mut out = Vec::with_capacity(logs.len());
        for log in logs {
            // topic2 is the indexed `bytes32 recordType`; `allowed` is the sole unindexed member.
            let Some(record_type) = log.topics().get(2) else {
                continue;
            };
            let allowed = bool::abi_decode(log.data().data.as_ref(), true)
                .map_err(|e| ChainError::Other(format!("approval log decode: {e}")))?;
            out.push((
                format!("0x{}", hex::encode(record_type.as_slice())),
                allowed,
            ));
        }
        Ok(out)
    }
}

/// Helper: normalize a record-type string into its keccak256 bytes32 (the whitelist / issuer key).
pub fn record_type_key(record_type: &str) -> String {
    use alloy::primitives::keccak256;
    let h: FixedBytes<32> = keccak256(record_type.as_bytes());
    format!("0x{}", hex::encode(h.as_slice()))
}

/// The purpose label reduced to the registry's bytes32 `purpose` field: keccak256(label) reduced mod
/// the BN254 scalar field r (a field element, distinct from recordType). MUST byte-match the vet
/// stack's `verify::purpose_key` and the on-chain `_verifyKey` input. (Mirrors stacks/vet/api verify.rs.)
pub fn purpose_key(label: &str) -> String {
    use alloy::primitives::{keccak256, U256};
    // BN254 r.
    let r = U256::from_str_radix(
        "21888242871839275222246405745257275088548364400416034343698204186575808495617",
        10,
    )
    .unwrap();
    let full = U256::from_be_bytes::<32>(keccak256(label.as_bytes()).0);
    let reduced = full % r;
    format!("0x{}", hex::encode(reduced.to_be_bytes::<32>()))
}

/// The IssuerRegistry whitelist key the VerificationRegistry checks for the relayer on a given purpose:
/// `keccak256(abi.encode("VERIFY:", purpose))` where `purpose` is the bytes32 from `purpose_key(label)`
/// (Solidity `abi.encode(string,bytes32)` = head[offset=0x40] ++ purpose ++ len(7) ++ "VERIFY:" padded).
/// MUST byte-match the on-chain `VerificationRegistry._verifyKey` + the vet stack's `verify::verify_key`.
/// (Mirrors stacks/vet/api verify.rs ~47-68.)
pub fn verify_key(label: &str) -> String {
    use alloy::primitives::keccak256;
    let purpose_hex = purpose_key(label);
    let purpose = hex::decode(purpose_hex.trim_start_matches("0x")).unwrap_or_default();
    // abi.encode(string "VERIFY:", bytes32 purpose)
    let mut buf = Vec::with_capacity(160);
    // [0] offset to the string data = 0x40 (after the two head words).
    let mut off = [0u8; 32];
    off[31] = 0x40;
    buf.extend_from_slice(&off);
    // [1] the bytes32 purpose word.
    buf.extend_from_slice(&purpose);
    // [2] string length = 7 ("VERIFY:").
    let mut len = [0u8; 32];
    len[31] = 7;
    buf.extend_from_slice(&len);
    // [3] string bytes, right-padded to 32.
    let mut data = [0u8; 32];
    data[..7].copy_from_slice(b"VERIFY:");
    buf.extend_from_slice(&data);
    format!("0x{}", hex::encode(keccak256(&buf).as_slice()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `verify_key` must byte-match the on-chain `_verifyKey` + the demo-bootstrap value the vet stack
    /// produces for "boarding_intake" — the verifier-onboarding whitelist parity guard (plan A3).
    #[test]
    fn verify_key_parity_boarding_intake() {
        assert_eq!(
            verify_key("boarding_intake"),
            "0x9f894293e0cbaa46eca3cc026ad45e5012c10c4d3217ede0488ca0d2b5eaf764"
        );
    }

    /// `purpose_key` is the bytes32 field element fed into `verify_key`, the relayer broadcast, and the
    /// nullifier. It MUST byte-match the vet stack's `verify::purpose_key` for the same label; this anchor
    /// is the parity guard (the matching value lives in the vet stack's verify.rs tests).
    #[test]
    fn purpose_key_parity_boarding_intake() {
        assert_eq!(
            purpose_key("boarding_intake"),
            "0x0d35de973921c6fca6d7ad626fe13c4017a093733a6a21689b631b2c61b1c18d"
        );
    }

    /// `purpose_key` must always be a 32-byte field element strictly less than the BN254 scalar field r,
    /// since it is reduced `mod r` before use as a circuit/registry input.
    #[test]
    fn purpose_key_is_reduced_field_element() {
        use alloy::primitives::U256;
        let r = U256::from_str_radix(
            "21888242871839275222246405745257275088548364400416034343698204186575808495617",
            10,
        )
        .unwrap();
        for label in [
            "",
            "boarding_intake",
            "grooming",
            "a-very-long-purpose-label-xyz",
        ] {
            let hex = purpose_key(label);
            assert_eq!(hex.len(), 66, "{label}: want 0x + 64 hex chars");
            let v = U256::from_str_radix(hex.trim_start_matches("0x"), 16).unwrap();
            assert!(
                v < r,
                "{label}: purpose_key must be reduced mod the BN254 field r"
            );
        }
    }

    /// `record_type_key` is the raw keccak256 of the label (NOT reduced mod r), so the empty string
    /// anchors to the well-known `keccak256("")`. Because that value exceeds the BN254 field r it gets
    /// reduced by `purpose_key`, so the two keys diverge for the empty label.
    #[test]
    fn record_type_key_anchors_and_differs_from_purpose() {
        assert_eq!(
            record_type_key(""),
            "0xc5d2460186f7233c927e7db2dcc703c0e500b653ca82273b7bfad8045d85a470"
        );
        // keccak256("") > r, so reduction is observable: raw recordType key != reduced purpose key.
        assert_ne!(record_type_key(""), purpose_key(""));
        // Deterministic and 0x + 64 hex chars.
        assert_eq!(record_type_key("grooming"), record_type_key("grooming"));
        assert_eq!(record_type_key("grooming").len(), 66);
    }

    /// `create_issuer_calldata` selects the correct 4-byte selector and is deterministic for fixed args.
    #[test]
    fn create_issuer_calldata_selector_and_determinism() {
        let rt = record_type_key("VACCINATION");
        let biz = "0x00000000000000000000000000000000000000ad";
        let a = create_issuer_calldata("Vax Authority", &rt, biz);
        let b = create_issuer_calldata("Vax Authority", &rt, biz);
        assert_eq!(a, b, "calldata must be deterministic");
        // selector = keccak256("createIssuer(string,bytes32,address)")[..4].
        use alloy::primitives::keccak256;
        let sel = keccak256(b"createIssuer(string,bytes32,address)");
        assert_eq!(&a[2..10], &hex::encode(&sel.as_slice()[..4]));
    }

    /// The role-key helpers anchor to their canonical values.
    #[test]
    fn role_key_anchors() {
        use alloy::primitives::keccak256;
        assert_eq!(
            whitelist_admin_role(),
            format!(
                "0x{}",
                hex::encode(keccak256(b"WHITELIST_ADMIN").as_slice())
            )
        );
        assert_eq!(default_admin_role(), format!("0x{}", "0".repeat(64)));
    }

    /// MemChain's clone preview is deterministic per (factory, recordType, business) and diverges when
    /// any component changes — the property the deploy preview relies on.
    #[tokio::test]
    async fn mem_predict_issuer_is_deterministic_and_input_sensitive() {
        let c = MemChain::new();
        let factory = "0x00000000000000000000000000000000000000fa";
        let rt = record_type_key("VACCINATION");
        let biz = "0x00000000000000000000000000000000000000ad";
        let p1 = c.predict_issuer(factory, &rt, biz).await.unwrap();
        let p2 = c.predict_issuer(factory, &rt, biz).await.unwrap();
        assert_eq!(p1, p2);
        assert!(p1.starts_with("0x") && p1.len() == 42);
        // different recordType -> different address.
        let p3 = c
            .predict_issuer(factory, &record_type_key("DOG_PROFILE"), biz)
            .await
            .unwrap();
        assert_ne!(p1, p3);
    }

}

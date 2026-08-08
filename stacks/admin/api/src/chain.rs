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

use crate::provider_registry::{
    IdentityAnchor, ProviderRecord, ResolverKind, ServiceEffective, ServiceRecord, Standing,
};

pub const ROAX_CHAIN_ID: u64 = 135;

sol! {
    #[sol(rpc)]
    contract IDogTagSBT {
        // AccessControl surface — the DEFAULT_ADMIN holder grants ISSUER_ROLE to owner-hidden issuers.
        function grantRole(bytes32 role, address account) external;
        function revokeRole(bytes32 role, address account) external;
        function hasRole(bytes32 role, address account) external view returns (bool);
        // AccessControlEnumerable — the mint-role card's "who holds it" read.
        function getRoleMemberCount(bytes32 role) external view returns (uint256);
        function getRoleMember(bytes32 role, uint256 index) external view returns (address);
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

        // The four writes that complete the journey past "the provider deployed a contract".
        //
        // `attachService` takes NO claimed factory or record type: it resolves the pinned factory
        // from `generationId`, proves `factory.isClone(service)`, then reads immutable `recordType()`
        // and live `owner()` off the service itself. `expectedOwner` is a transaction guard against a
        // second ownership handover between review and send - the RESOLVED owner stays authoritative,
        // so this argument can only refuse a send, never choose what is attached.
        function attachService(
            bytes20 providerId,
            address serviceAddress,
            bytes32 generationId,
            address expectedOwner
        ) external;
        function setServiceStanding(address serviceAddress, uint8 newStanding) external;
        // Rights are keyed on the ADDRESS and carry no service: at approval time the applicant has
        // no clone, so there is nothing to key a grant against. `rights` is the account's COMPLETE
        // settable mask afterwards, so withdrawing everything is `setRights(account, 0)`.
        function setRights(address account, uint256 rights) external;
        // NOTE the first argument is the RAW `purpose`, not its verification key: the contract
        // derives `verificationKey(purpose) = keccak256(abi.encode("VERIFY:", purpose))` itself
        // (ProviderRegistry.sol:786-793). Passing an already-derived key here double-derives and
        // writes the capability under a key nothing reads - silently, since the write succeeds.
        function setVerifierCapability(bytes32 purpose, address relayer, bool allowed) external;
        function setResolverApproved(uint8 kind, address resolver, bool approved) external;

        function provider(bytes20 providerId) external view returns (Provider memory);
        function publicIdentityAnchor(bytes20 providerId) external view returns (PublicIdentityAnchor memory);
        function providerCount() external view returns (uint256);
        function providerPage(uint256 cursor, uint256 limit)
            external
            view
            returns (bytes20[] memory values, uint256 nextCursor);
        function owner() external view returns (address);

        function service(address serviceAddress) external view returns (Service memory);
        function providerServicePage(bytes20 providerId, uint256 cursor, uint256 limit)
            external
            view
            returns (address[] memory values, uint256 nextCursor);
        // The five lifecycle terms `canIssue` folds, reported SEPARATELY. Pre-ANDing them into one
        // bool would leave an admin unable to tell a provider suspension from an unconfirmed owner
        // handover, which have entirely different remedies.
        function effectiveService(address serviceAddress)
            external
            view
            returns (
                uint8 providerStanding,
                uint8 serviceStanding,
                bool factoryActive,
                bool ownerConfirmed,
                bool hasActiveIssuer
            );
        function currentService(bytes20 providerId, bytes32 recordType) external view returns (address);
        // THE rights lookup: one address in, one bitmask out. No service, no record type, no caller
        // context. Bit 0 is `RIGHT_ISSUE`; see `dogtag_standard::verify::RIGHT_ISSUE`.
        function rightsOf(address account) external view returns (uint256);
        // KEPT while `whitelistFor`/`delistFor` were deleted, and the difference is not a nuance:
        // `ProviderRegistry` DOES implement this one, answering the orthogonal VERIFY axis for a
        // caller that is not itself an attached service. Never hand it a RECORD-TYPE key - the
        // answer would be a confident `false` about every genuine issuer signer.
        function isWhitelistedFor(bytes32 key, address signer) external view returns (bool);
        function issueRightHolders() external view returns (uint256);
        function canVerify(bytes32 purpose, address relayer) external view returns (bool);
        function factoryGeneration(bytes32 generationId) external view returns (FactoryGeneration memory);
        function factoryGenerationPage(uint256 cursor, uint256 limit)
            external
            view
            returns (bytes32[] memory values, uint256 nextCursor);
        function isResolverApproved(uint8 kind, address resolver) external view returns (bool);
        function resolverPage(uint8 kind, uint256 cursor, uint256 limit)
            external
            view
            returns (address[] memory values, uint256 nextCursor);

        // The ONLY direct evidence of what a provider is approved for - `_serviceCreationApprovals`
        // is private with no getter. Both leading args are indexed.
        event ServiceCreationApprovalSet(bytes20 indexed providerId, bytes32 indexed recordType, bool allowed);
        // Likewise: `rightsOf(account)` is a POINT read and `_grantedRights` has no enumeration, so
        // WHO holds a right is only knowable from these logs. `rights` is the whole settable mask, so
        // the last event for an account IS its current mask.
        event RightsSet(address indexed account, uint256 rights);
        // THREE indexed args: the group is topic1 (`purpose`) and the subject is topic3 (`relayer`);
        // topic2 is the derived compatibility key. Reading the subject off topic2 yields a bytes32
        // rendered as an address, which is a plausible-looking value and never a real relayer.
        event VerifierCapabilitySet(
            bytes32 indexed purpose, bytes32 indexed compatibilityKey, address indexed relayer, bool allowed
        );
        event ResolverApprovalSet(uint8 indexed kind, address indexed resolver, bool approved);
    }

    // The two immutable-ish facts `attachService` resolves off the service itself. Probing them
    // BEFORE sending is what turns `InvalidServiceMetadata()` into a sentence an admin can act on -
    // most importantly for a generation-1 `DogTagIssuer`, which is `Initializable` only and has no
    // `owner()` at all, so it can never be attached however correct the rest of the form is.
    #[sol(rpc)]
    contract IServiceProbe {
        function owner() external view returns (address);
        function recordType() external view returns (bytes32);
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

    // `ProviderRegistry.Service` (ProviderRegistry.sol:105-113) and `FactoryGeneration` (:94-99).
    // Member ORDER is the ABI: a member out of place decodes silently into the wrong field, so a
    // `recordType` read as a `confirmedOwner` would render as a plausible address rather than error.
    struct Service {
        bytes20 providerId;
        bytes32 factoryGeneration;
        bytes32 recordType;
        address confirmedOwner;
        address domainResolver;
        uint64 ownerEpoch;
        uint8 standing;
    }

    struct FactoryGeneration {
        address factory;
        bool active;
        uint64 addedAtBlock;
        uint64 deprecatedAtBlock;
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

    /// `ProviderRegistry.setRights(account, rights)` — `onlyOwner` write, the issuer-application
    /// approval path. `rights` is the account's COMPLETE settable mask afterwards, so a withdrawal is
    /// `rights = 0`.
    ///
    /// It takes NO record type, because the grant carries none. That is what makes it callable at
    /// approval time, when the applicant has no clone for a per-service grant to name.
    async fn set_rights(
        &self,
        account_index: u32,
        registry_addr: &str,
        account: &str,
        rights: u64,
    ) -> Result<SentTx, ChainError>;

    /// `ProviderRegistry.setVerifierCapability(purpose, relayer, allowed)` — `onlyOwner` write.
    ///
    /// `purpose` is the RAW bytes32 from `purpose_key`, NOT `verify_key`: the contract derives
    /// `verificationKey` itself, and an already-derived key derives twice and grants nothing.
    async fn set_verifier_capability(
        &self,
        account_index: u32,
        registry_addr: &str,
        purpose: &str,
        relayer: &str,
        allowed: bool,
    ) -> Result<SentTx, ChainError>;

    /// `ProviderRegistry.isWhitelistedFor(key, signer)` — the orthogonal VERIFY axis, and the one
    /// member of the retired `whitelistFor`/`delistFor`/`isWhitelistedFor` trio the launch set still
    /// implements. `key` is a VERIFY key, never a record-type key.
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

    /// Every current ISSUER_ROLE holder, via `AccessControlEnumerable` (`getRoleMemberCount` +
    /// `getRoleMember`). The mint-role card's "who holds it" — an EMPTY list is the exact
    /// misprovisioning the 2026-08-07 live walk hit, so it must be a definite answer, apart from a
    /// failed read.
    async fn issuer_role_holders(&self, sbt_addr: &str) -> Result<Vec<String>, ChainError>;

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

    /// `ProviderRegistry.provider(providerId)`. Does NOT revert for an unknown id - it answers a
    /// zero-filled struct - so `ProviderRecord::registered` (`controller != 0`) is the existence
    /// test, never the standing.
    async fn provider_record(
        &self,
        registry_addr: &str,
        provider_id: &str,
    ) -> Result<ProviderRecord, ChainError>;

    /// `ProviderRegistry.publicIdentityAnchor(providerId)` - the registrar's own identity assertion.
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

    /// The same log for EVERY provider on one registry, grouped by `providerId`, each group in
    /// ascending `(block, logIndex)` order.
    ///
    /// One unbounded `eth_getLogs` for the whole registry rather than one PER PROVIDER. That is not
    /// only a cost argument: a per-provider scan can leave a page MIXED - early providers resolved,
    /// later ones `Unavailable` after a rate-limiting or range-capping peer starts refusing - which
    /// reads as a fact about those particular providers when the truth is one uniform "we could not
    /// check". With a single read the whole page resolves or the whole page does not.
    ///
    /// A provider absent from the returned map has no log entries, which IS an answer (the read
    /// resolved and mentions it nowhere), never a could-not-check.
    async fn service_creation_approval_log_by_provider(
        &self,
        registry_addr: &str,
    ) -> Result<HashMap<String, Vec<(String, bool)>>, ChainError>;

    /// `ProviderRegistry.service(serviceAddress)`. Like `provider()`, answers a zero-filled struct
    /// for an address it has never seen, so `ServiceRecord::attached` (`providerId != 0`) is the
    /// existence test rather than the standing.
    async fn service_record(
        &self,
        registry_addr: &str,
        service_addr: &str,
    ) -> Result<ServiceRecord, ChainError>;

    /// `ProviderRegistry.providerServicePage(providerId, cursor, limit)` → `(services, nextCursor)`.
    async fn provider_service_page(
        &self,
        registry_addr: &str,
        provider_id: &str,
        cursor: u64,
        limit: u64,
    ) -> Result<(Vec<String>, u64), ChainError>;

    /// `ProviderRegistry.effectiveService(serviceAddress)` - the five lifecycle terms, kept apart.
    async fn service_effective(
        &self,
        registry_addr: &str,
        service_addr: &str,
    ) -> Result<ServiceEffective, ChainError>;

    /// `ProviderRegistry.currentService(providerId, recordType)` - the provider's published pointer
    /// for that record type, or the zero address when it has selected none.
    async fn current_service(
        &self,
        registry_addr: &str,
        provider_id: &str,
        record_type_key: &str,
    ) -> Result<String, ChainError>;

    /// `ProviderRegistry.factoryGenerationPage(cursor, limit)` paired with `factoryGeneration(id)`,
    /// as `(generationId, factory, active)`. Attachment needs the generation whose pinned factory
    /// recognizes the clone, and an admin must never be asked to type a bytes32 for it.
    async fn factory_generations(
        &self,
        registry_addr: &str,
    ) -> Result<Vec<(String, String, bool)>, ChainError>;

    /// `DogTagIssuer.owner()` and `.recordType()` off the service itself - the two facts
    /// `attachService` resolves. A generation-1 clone has no `owner()`, so this read FAILS for one,
    /// which is the honest way to say it can never be attached.
    async fn service_metadata(&self, service_addr: &str) -> Result<(String, String), ChainError>;

    /// The `RightsSet` log for the whole registry, ascending `(block, logIndex)`, as
    /// `(account, holds_issue)` pairs. `rightsOf` is a POINT read with no enumeration, so this log is
    /// the only way to learn WHO holds a right; a failed read is never an empty set.
    ///
    /// It takes NO service, because a grant carries none. A caller that wants "who may issue on this
    /// service" is asking a question that no longer has a per-service answer: every holder may, on
    /// every service in effective standing.
    async fn issuance_rights_log(
        &self,
        registry_addr: &str,
    ) -> Result<Vec<(String, bool)>, ChainError>;

    /// The `VerifierCapabilitySet` log for one purpose, ascending, as `(relayer, allowed)` pairs.
    /// Keyed by PURPOSE and not by service: the verify axis is orthogonal to issuance.
    async fn verifier_capability_log(
        &self,
        registry_addr: &str,
        purpose: &str,
    ) -> Result<Vec<(String, bool)>, ChainError>;

    /// `ProviderRegistry.resolverPage(kind, ...)` folded against `isResolverApproved(kind, r)`.
    ///
    /// The list is append-only and RETAINS a deapproved resolver, so the approval flag is read
    /// per address rather than inferred from listing: a resolver that was approved and then pulled
    /// is a different fact from one that was never approved, and only the flag separates them.
    async fn approved_resolvers(
        &self,
        registry_addr: &str,
        kind: ResolverKind,
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
    /// (provider_registry_addr, service_addr) -> the attached service record.
    services: HashMap<(String, String), ServiceRecord>,
    /// Attachment order per (registry, provider), mirroring `_providerServices`.
    provider_services: HashMap<(String, String), Vec<String>>,
    /// (registry, provider_id, record_type_key) -> the provider's published current pointer.
    current_services: HashMap<(String, String, String), String>,
    /// (registry, service_addr) -> the ordered `IssuanceCapabilitySet` log.
    /// registry_addr -> the ordered `RightsSet` log, as `(account, holds_issue)`.
    issuance_log: HashMap<String, Vec<(String, bool)>>,
    /// (registry, purpose) -> the ordered `VerifierCapabilitySet` log.
    verifier_log: HashMap<(String, String), Vec<(String, bool)>>,
    /// (registry, kind) -> resolver approval state, in first-listed order like `_resolverAddresses`.
    resolvers: HashMap<(String, u8), Vec<(String, bool)>>,
    /// (registry, generation_id) -> (factory, active).
    factory_generations: HashMap<(String, String), (String, bool)>,
    /// Listing order per registry, mirroring `_factoryGenerationIds`.
    factory_generation_ids: HashMap<String, Vec<String>>,
    /// service_addr -> (owner, recordTypeKey) as the clone itself answers. An address ABSENT here
    /// models a contract with no `owner()` - which is every generation-1 `DogTagIssuer`, and the one
    /// state `attachService` must refuse.
    service_metadata: HashMap<String, (String, String)>,
    /// Registries whose reads are forced to fail, so a route's could-not-run arm is reachable. A fake
    /// that cannot fail cannot exercise the state that exists for failure.
    failing_reads: std::collections::HashSet<String>,
    /// Registries whose APPROVAL LOG read alone is forced to fail, leaving `provider()` answering.
    failing_approval_log_reads: std::collections::HashSet<String>,
    /// Registries whose CAPABILITY LOG reads alone are forced to fail.
    ///
    /// Its own switch, not a share of `failing_reads`: the realistic failure is the service `eth_call`
    /// answering while a range-capping peer refuses the capability `eth_getLogs`, and a shared switch
    /// short-circuits into a read error before the `unavailable` arm is ever built - leaving the
    /// three-state claim unpinned at the layer that produces it.
    failing_capability_log_reads: std::collections::HashSet<String>,
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

    /// Fail ONLY the `ServiceCreationApprovalSet` log read, leaving `provider()` answering.
    ///
    /// A SEPARATE switch, not a mode of the one above, for the same reason vet-api needs
    /// `set_fail_find_pets_by_dog_tag_reads` beside its sibling: a shared switch cannot express
    /// "the provider read is fine and only this one fails", because whichever read comes first
    /// short-circuits and the route never reaches the arm under test.
    ///
    /// It is also the REALISTIC failure rather than a contrived one. `provider()` is an `eth_call`
    /// while this is an `eth_getLogs` from genesis to latest, and a rate-limited or range-capping
    /// peer refuses exactly the second while answering the first - the same asymmetry this repo
    /// records for the issuer-whitelist pillar's unbounded log reads.
    pub fn set_failing_approval_log_reads(&self, registry_addr: &str, failing: bool) {
        let mut g = self.inner.lock().unwrap();
        if failing {
            g.failing_approval_log_reads
                .insert(registry_addr.to_lowercase());
        } else {
            g.failing_approval_log_reads
                .remove(&registry_addr.to_lowercase());
        }
    }

    /// Fail ONLY the capability log reads (issuance / verifier / resolver), leaving `service()` and
    /// `provider()` answering - so a route's `CapabilitiesRead::Unavailable` arm is reachable while
    /// the rest of the view resolves. See `set_failing_approval_log_reads` for why this is its own
    /// switch rather than a mode of `set_failing_provider_reads`.
    pub fn set_failing_capability_log_reads(&self, registry_addr: &str, failing: bool) {
        let mut g = self.inner.lock().unwrap();
        if failing {
            g.failing_capability_log_reads
                .insert(registry_addr.to_lowercase());
        } else {
            g.failing_capability_log_reads
                .remove(&registry_addr.to_lowercase());
        }
    }

    /// Seed a factory's clone set, as `createIssuer` would.
    ///
    /// Needed because the registrar attaches contracts the PROVIDER deployed on their own portal,
    /// which is a different actor from the admin factory route - so `create_issuer` is not the path
    /// that puts them there. It doubles as the fake's "this address has code" set, which is what
    /// `setResolverApproved` refuses a non-contract on.
    pub fn set_clone(&self, factory_addr: &str, clone_addr: &str) {
        self.inner
            .lock()
            .unwrap()
            .clones
            .insert((factory_addr.to_lowercase(), clone_addr.to_lowercase()));
    }

    /// Seed what a service contract itself answers for `owner()` and `recordType()`.
    ///
    /// An address left UNSEEDED models a contract that answers neither - which is every
    /// generation-1 `DogTagIssuer`, since it is `Initializable` only and has no owner at all. That
    /// is the single most likely thing an admin will try to attach, so it must be reachable here.
    pub fn set_service_metadata(&self, service_addr: &str, owner: &str, record_type_key: &str) {
        self.inner.lock().unwrap().service_metadata.insert(
            service_addr.to_lowercase(),
            (owner.to_lowercase(), record_type_key.to_lowercase()),
        );
    }

    /// Seed a factory generation and the factory it pins, as `addFactoryGeneration` would.
    pub fn set_factory_generation(
        &self,
        registry_addr: &str,
        generation_id: &str,
        factory_addr: &str,
        active: bool,
    ) {
        let mut g = self.inner.lock().unwrap();
        let registry = registry_addr.to_lowercase();
        let gid = generation_id.to_lowercase();
        if g.factory_generations
            .insert(
                (registry.clone(), gid.clone()),
                (factory_addr.to_lowercase(), active),
            )
            .is_none()
        {
            g.factory_generation_ids
                .entry(registry)
                .or_default()
                .push(gid);
        }
    }

    /// Seed a provider's published current pointer, as the provider's own `repointService` would.
    /// It is the PROVIDER's decision, never the registrar's, so no registrar route writes it.
    pub fn set_current_service(
        &self,
        registry_addr: &str,
        provider_id: &str,
        record_type_key: &str,
        service_addr: &str,
    ) {
        self.inner.lock().unwrap().current_services.insert(
            (
                registry_addr.to_lowercase(),
                provider_id.to_lowercase(),
                record_type_key.to_lowercase(),
            ),
            service_addr.to_lowercase(),
        );
    }

    fn provider_reads_fail(g: &MemChainInner, registry_addr: &str) -> Result<(), ChainError> {
        if g.failing_reads.contains(&registry_addr.to_lowercase()) {
            return Err(ChainError::Rpc("provider read failed (seeded)".into()));
        }
        Ok(())
    }

    fn capability_log_reads_fail(g: &MemChainInner, registry_addr: &str) -> Result<(), ChainError> {
        if g.failing_capability_log_reads
            .contains(&registry_addr.to_lowercase())
        {
            return Err(ChainError::Rpc(
                "capability log read failed (seeded)".into(),
            ));
        }
        Ok(())
    }

    fn approval_log_reads_fail(g: &MemChainInner, registry_addr: &str) -> Result<(), ChainError> {
        if g.failing_approval_log_reads
            .contains(&registry_addr.to_lowercase())
        {
            return Err(ChainError::Rpc(
                "eth_getLogs failed: query returned more than 10000 results (seeded)".into(),
            ));
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

    // DogTagSBTConsent role writes (the mint-role card). OZ AccessControl semantics: grant and
    // revoke are IDEMPOTENT (no revert on a no-op; the route's own pre-check is what refuses one),
    // and only the ISSUER_ROLE key is modelled — an unrelated role neither mutates nor errors.
    if let Ok(c) = IDogTagSBT::grantRoleCall::abi_decode(&data, true) {
        if format!("0x{}", hex::encode(c.role.as_slice())) == issuer_role_key() {
            let who = format!("{:#x}", c.account);
            g.issuer_roles.insert((registry.clone(), who));
        }
        return Ok(());
    }
    if let Ok(c) = IDogTagSBT::revokeRoleCall::abi_decode(&data, true) {
        if format!("0x{}", hex::encode(c.role.as_slice())) == issuer_role_key() {
            let who = format!("{:#x}", c.account);
            g.issuer_roles.remove(&(registry.clone(), who));
        }
        return Ok(());
    }

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
        g.provider_ids
            .entry(registry.clone())
            .or_default()
            .push(id.clone());
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
            return Err(ChainError::Other(
                "tx reverted: InvalidServiceMetadata".into(),
            ));
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

    if let Ok(c) = IProviderRegistry::attachServiceCall::abi_decode(&data, true) {
        let id = format!("0x{}", hex::encode(c.providerId.as_slice()));
        let service = format!("{:#x}", c.serviceAddress).to_lowercase();
        let gid = format!("0x{}", hex::encode(c.generationId.as_slice()));
        if !g.providers.contains_key(&(registry.clone(), id.clone())) {
            return Err(ChainError::Other("tx reverted: UnknownProvider".into()));
        }
        if c.serviceAddress.is_zero() || c.expectedOwner.is_zero() {
            return Err(ChainError::Other("tx reverted: ZeroAddress".into()));
        }
        if g.services
            .contains_key(&(registry.clone(), service.clone()))
        {
            return Err(ChainError::Other("tx reverted: AlreadyRegistered".into()));
        }
        let Some((factory, active)) = g
            .factory_generations
            .get(&(registry.clone(), gid.clone()))
            .cloned()
        else {
            return Err(ChainError::Other(
                "tx reverted: UnknownFactoryGeneration".into(),
            ));
        };
        if !active {
            return Err(ChainError::Other(
                "tx reverted: FactoryGenerationInactive".into(),
            ));
        }
        if !g.clones.contains(&(factory, service.clone())) {
            return Err(ChainError::Other("tx reverted: NotFactoryClone".into()));
        }
        // An unseeded service answers neither `owner()` nor `recordType()`, which is exactly a
        // generation-1 clone. The contract folds that into `InvalidServiceMetadata()`.
        let Some((live_owner, record_type_key)) = g.service_metadata.get(&service).cloned() else {
            return Err(ChainError::Other(
                "tx reverted: InvalidServiceMetadata".into(),
            ));
        };
        if live_owner == zero_addr()
            || record_type_key
                .trim_start_matches("0x")
                .chars()
                .all(|ch| ch == '0')
        {
            return Err(ChainError::Other(
                "tx reverted: InvalidServiceMetadata".into(),
            ));
        }
        if !live_owner.eq_ignore_ascii_case(&format!("{:#x}", c.expectedOwner)) {
            return Err(ChainError::Other(
                "tx reverted: UnexpectedServiceOwner".into(),
            ));
        }
        g.services.insert(
            (registry.clone(), service.clone()),
            ServiceRecord {
                service_address: service.clone(),
                provider_id: id.clone(),
                factory_generation: gid,
                record_type: crate::provider_registry::record_type_label(&record_type_key),
                record_type_key,
                // The RESOLVED owner is stored, never the caller's expected value - the guard can
                // refuse a send and can never choose what is written.
                confirmed_owner: live_owner,
                domain_resolver: zero_addr(),
                owner_epoch: 1,
                // Attachment lands the service at PENDING, exactly as registration does a provider.
                // Modelling it is what makes the missing standing step visible in a test rather
                // than only on a live chain.
                standing: Standing::Pending,
                attached: true,
            },
        );
        g.provider_services
            .entry((registry, id))
            .or_default()
            .push(service);
        return Ok(());
    }

    if let Ok(c) = IProviderRegistry::setServiceStandingCall::abi_decode(&data, true) {
        let service = format!("{:#x}", c.serviceAddress).to_lowercase();
        let rec = g
            .services
            .get_mut(&(registry, service))
            .ok_or_else(|| ChainError::Other("tx reverted: UnknownService".into()))?;
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

    if let Ok(c) = IProviderRegistry::setRightsCall::abi_decode(&data, true) {
        if c.account.is_zero() {
            return Err(ChainError::Other("tx reverted: ZeroAddress".into()));
        }
        // The contract refuses any bit outside `SETTABLE_RIGHTS`, and today that is `RIGHT_ISSUE`
        // alone. A fake that accepted a derived bit would let a test assert a grant the chain
        // refuses.
        let settable = U256::from(dogtag_standard::verify::RIGHT_ISSUE);
        if c.rights & !settable != U256::ZERO {
            return Err(ChainError::Other("tx reverted: UnsettableRights".into()));
        }
        // NOTE: deliberately NO service existence check. The whole point of the re-keying is that a
        // grant can be made before the applicant has a clone.
        let account = format!("{:#x}", c.account).to_lowercase();
        let holds_issue = c.rights & settable != U256::ZERO;
        let log = g.issuance_log.entry(registry).or_default();
        let current = last_bool_for(log, &account);
        if current == holds_issue {
            return Err(ChainError::Other("tx reverted: NoChange".into()));
        }
        log.push((account, holds_issue));
        return Ok(());
    }

    if let Ok(c) = IProviderRegistry::setVerifierCapabilityCall::abi_decode(&data, true) {
        if c.relayer.is_zero() {
            return Err(ChainError::Other("tx reverted: ZeroAddress".into()));
        }
        let purpose = format!("0x{}", hex::encode(c.purpose.as_slice()));
        let relayer = format!("{:#x}", c.relayer).to_lowercase();
        let log = g.verifier_log.entry((registry, purpose)).or_default();
        let current = last_bool_for(log, &relayer);
        if current == c.allowed {
            return Err(ChainError::Other("tx reverted: NoChange".into()));
        }
        log.push((relayer, c.allowed));
        return Ok(());
    }

    if let Ok(c) = IProviderRegistry::setResolverApprovedCall::abi_decode(&data, true) {
        if c.resolver.is_zero() {
            return Err(ChainError::Other("tx reverted: ZeroAddress".into()));
        }
        let resolver = format!("{:#x}", c.resolver).to_lowercase();
        // `approved && resolver.code.length == 0` reverts `ResolverNotApproved()`: only a CONTRACT
        // may be approved. `clones` is the fake's stand-in for "this address has code".
        if c.approved && !g.clones.iter().any(|(_, addr)| addr == &resolver) {
            return Err(ChainError::Other("tx reverted: ResolverNotApproved".into()));
        }
        let list = g.resolvers.entry((registry, c.kind)).or_default();
        match list.iter_mut().find(|(addr, _)| addr == &resolver) {
            Some(slot) => {
                if slot.1 == c.approved {
                    return Err(ChainError::Other("tx reverted: NoChange".into()));
                }
                slot.1 = c.approved;
            }
            None => {
                if !c.approved {
                    return Err(ChainError::Other("tx reverted: NoChange".into()));
                }
                // Append-only listing: a deapproved resolver KEEPS its slot, so "approved then
                // pulled" stays distinguishable from "never approved".
                list.push((resolver, true));
            }
        }
        return Ok(());
    }

    Ok(())
}

/// The current state of one holder in a `(holder, allowed)` log: the LAST write wins, and an
/// address the log never mentions has never been granted.
fn last_bool_for(log: &[(String, bool)], holder: &str) -> bool {
    log.iter()
        .rev()
        .find(|(h, _)| h.eq_ignore_ascii_case(holder))
        .map(|(_, allowed)| *allowed)
        .unwrap_or(false)
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

    async fn set_rights(
        &self,
        account_index: u32,
        registry_addr: &str,
        account: &str,
        rights: u64,
    ) -> Result<SentTx, ChainError> {
        let mut g = self.inner.lock().unwrap();
        // emulate onlyOwner: require a registered admin signer at this index.
        g.signers
            .get(&account_index)
            .cloned()
            .ok_or_else(|| ChainError::Other("no admin signer for index".into()))?;
        // The contract refuses any bit outside `SETTABLE_RIGHTS`, so the fake does too. Without this
        // the fake would accept a mask the chain rejects, and a route that sent one would look fine
        // here and revert on ROAX.
        if rights & !dogtag_standard::verify::RIGHT_ISSUE != 0 {
            return Err(ChainError::Other("tx reverted: UnsettableRights".into()));
        }
        let holds_issue = rights & dogtag_standard::verify::RIGHT_ISSUE != 0;
        let account = account.to_lowercase();
        let log = g
            .issuance_log
            .entry(registry_addr.to_lowercase())
            .or_default();
        // The real contract refuses a no-op with `NoChange()`. Modelled, so a route that would send
        // a reverting transaction fails here too rather than reporting a grant that never landed.
        if last_bool_for(log, &account) == holds_issue {
            return Err(ChainError::Other("tx reverted: NoChange".into()));
        }
        log.push((account, holds_issue));
        let tx_hash = Self::next_tx(&mut g);
        Ok(SentTx { tx_hash })
    }

    async fn set_verifier_capability(
        &self,
        account_index: u32,
        registry_addr: &str,
        purpose: &str,
        relayer: &str,
        allowed: bool,
    ) -> Result<SentTx, ChainError> {
        let mut g = self.inner.lock().unwrap();
        g.signers
            .get(&account_index)
            .cloned()
            .ok_or_else(|| ChainError::Other("no admin signer for index".into()))?;
        // Keyed on the RAW purpose word, exactly as this fake's own calldata decoder keys it
        // (`setVerifierCapabilityCall` above). The two must agree, or a grant made through this
        // method would be invisible to a read made after one made through a dispatched action.
        let purpose = purpose.to_lowercase();
        let relayer = relayer.to_lowercase();
        let log = g
            .verifier_log
            .entry((registry_addr.to_lowercase(), purpose))
            .or_default();
        if last_bool_for(log, &relayer) == allowed {
            return Err(ChainError::Other("tx reverted: NoChange".into()));
        }
        log.push((relayer, allowed));
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

    async fn issuer_role_holders(&self, sbt_addr: &str) -> Result<Vec<String>, ChainError> {
        let g = self.inner.lock().unwrap();
        let key = sbt_addr.to_lowercase();
        // Sorted for determinism — a HashSet's iteration order is not a wire contract.
        let mut holders: Vec<String> = g
            .issuer_roles
            .iter()
            .filter(|(sbt, _)| *sbt == key)
            .map(|(_, who)| who.clone())
            .collect();
        holders.sort();
        Ok(holders)
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
            // does - `registered: false` is the existence answer, never the standing.
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
        Self::approval_log_reads_fail(&g, registry_addr)?;
        Ok(g.approval_log
            .get(&(registry_addr.to_lowercase(), provider_id.to_lowercase()))
            .cloned()
            .unwrap_or_default())
    }

    async fn service_creation_approval_log_by_provider(
        &self,
        registry_addr: &str,
    ) -> Result<HashMap<String, Vec<(String, bool)>>, ChainError> {
        let g = self.inner.lock().unwrap();
        Self::provider_reads_fail(&g, registry_addr)?;
        // ONE read, so the same switch that fails the per-provider log fails this one - which is what
        // makes the whole page uniformly `Unavailable` rather than mixed.
        Self::approval_log_reads_fail(&g, registry_addr)?;
        let registry = registry_addr.to_lowercase();
        Ok(g.approval_log
            .iter()
            .filter(|((r, _), _)| *r == registry)
            .map(|((_, id), events)| (id.clone(), events.clone()))
            .collect())
    }

    async fn service_record(
        &self,
        registry_addr: &str,
        service_addr: &str,
    ) -> Result<ServiceRecord, ChainError> {
        let g = self.inner.lock().unwrap();
        Self::provider_reads_fail(&g, registry_addr)?;
        let key = (registry_addr.to_lowercase(), service_addr.to_lowercase());
        Ok(g.services.get(&key).cloned().unwrap_or_else(|| {
            // `service()` answers a zero-filled struct rather than reverting, so an unknown address
            // reads as "attached: false" and never as a read failure.
            ServiceRecord {
                service_address: service_addr.to_lowercase(),
                provider_id: format!("0x{}", "0".repeat(40)),
                factory_generation: format!("0x{}", "0".repeat(64)),
                record_type_key: format!("0x{}", "0".repeat(64)),
                record_type: None,
                confirmed_owner: zero_addr(),
                domain_resolver: zero_addr(),
                owner_epoch: 0,
                standing: Standing::None,
                attached: false,
            }
        }))
    }

    async fn provider_service_page(
        &self,
        registry_addr: &str,
        provider_id: &str,
        cursor: u64,
        limit: u64,
    ) -> Result<(Vec<String>, u64), ChainError> {
        let g = self.inner.lock().unwrap();
        Self::provider_reads_fail(&g, registry_addr)?;
        let empty = Vec::new();
        let all = g
            .provider_services
            .get(&(registry_addr.to_lowercase(), provider_id.to_lowercase()))
            .unwrap_or(&empty);
        // Mirrors `_checkPage`: `cursor == length` is legal and returns an empty page.
        if limit == 0 || cursor > all.len() as u64 {
            return Err(ChainError::Other("tx reverted: BadPage".into()));
        }
        let start = cursor as usize;
        let end = ((cursor + limit) as usize).min(all.len());
        Ok((all[start..end].to_vec(), end as u64))
    }

    async fn service_effective(
        &self,
        registry_addr: &str,
        service_addr: &str,
    ) -> Result<ServiceEffective, ChainError> {
        let g = self.inner.lock().unwrap();
        Self::provider_reads_fail(&g, registry_addr)?;
        let registry = registry_addr.to_lowercase();
        let service = service_addr.to_lowercase();
        let rec = g.services.get(&(registry.clone(), service.clone()));
        let provider_standing = rec
            .and_then(|s| g.providers.get(&(registry.clone(), s.provider_id.clone())))
            .map(|p| p.standing)
            .unwrap_or(Standing::None);
        let factory_active = rec
            .and_then(|s| {
                g.factory_generations
                    .get(&(registry.clone(), s.factory_generation.clone()))
            })
            .map(|(factory, active)| {
                *active && g.clones.contains(&(factory.clone(), service.clone()))
            })
            .unwrap_or(false);
        // The fake stores the RESOLVED owner at attachment and models no post-attach handover, so a
        // service that exists is owner-confirmed. `confirmServiceOwner` is a separate lever.
        let owner_confirmed = rec
            .map(|s| s.confirmed_owner != zero_addr())
            .unwrap_or(false);
        let service_standing = rec.map(|s| s.standing).unwrap_or(Standing::None);
        // `_issueRightHolders != 0` — a REGISTRY-WIDE count now, because a grant names no service.
        let any_grant = g
            .issuance_log
            .get(&registry)
            .map(|log| {
                let folded = crate::provider_registry::fold_capabilities(log);
                folded.iter().any(|e| e.allowed)
            })
            .unwrap_or(false);
        // `hasActiveIssuer` is `_issueRightHolders != 0 && _serviceIssuanceEligible(..)`, and that
        // predicate folds the confirmed owner, `_serviceStandingIsEffective` AND the provider's
        // current pointer - which only the provider's own `repointService` writes. A fake that
        // folded the grant alone would report a service as ready to issue in exactly the state the
        // registrar reaches when it has finished, where every issuance through it reverts.
        let standing_effective = rec.is_some()
            && service_standing == Standing::Active
            && provider_standing == Standing::Active
            && factory_active;
        let is_current = rec
            .map(|s| {
                g.current_services
                    .get(&(
                        registry,
                        s.provider_id.to_lowercase(),
                        s.record_type_key.to_lowercase(),
                    ))
                    .map(|c| c.eq_ignore_ascii_case(&service))
                    .unwrap_or(false)
            })
            .unwrap_or(false);
        Ok(ServiceEffective {
            provider_standing,
            service_standing,
            factory_active,
            owner_confirmed,
            has_active_issuer: any_grant && owner_confirmed && standing_effective && is_current,
        })
    }

    async fn current_service(
        &self,
        registry_addr: &str,
        provider_id: &str,
        record_type_key: &str,
    ) -> Result<String, ChainError> {
        let g = self.inner.lock().unwrap();
        Self::provider_reads_fail(&g, registry_addr)?;
        Ok(g.current_services
            .get(&(
                registry_addr.to_lowercase(),
                provider_id.to_lowercase(),
                record_type_key.to_lowercase(),
            ))
            .cloned()
            .unwrap_or_else(zero_addr))
    }

    async fn factory_generations(
        &self,
        registry_addr: &str,
    ) -> Result<Vec<(String, String, bool)>, ChainError> {
        let g = self.inner.lock().unwrap();
        Self::provider_reads_fail(&g, registry_addr)?;
        let registry = registry_addr.to_lowercase();
        Ok(g.factory_generation_ids
            .get(&registry)
            .map(|ids| {
                ids.iter()
                    .filter_map(|gid| {
                        g.factory_generations
                            .get(&(registry.clone(), gid.clone()))
                            .map(|(factory, active)| (gid.clone(), factory.clone(), *active))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn service_metadata(&self, service_addr: &str) -> Result<(String, String), ChainError> {
        let g = self.inner.lock().unwrap();
        g.service_metadata
            .get(&service_addr.to_lowercase())
            .cloned()
            // An unseeded address answers neither getter - a generation-1 clone, or an EOA. That is
            // a read FAILURE rather than a zero answer, because a staticcall to a contract without
            // the selector reverts.
            .ok_or_else(|| {
                ChainError::Rpc("the service answered neither owner() nor recordType()".into())
            })
    }

    async fn issuance_rights_log(
        &self,
        registry_addr: &str,
    ) -> Result<Vec<(String, bool)>, ChainError> {
        let g = self.inner.lock().unwrap();
        Self::provider_reads_fail(&g, registry_addr)?;
        Self::capability_log_reads_fail(&g, registry_addr)?;
        Ok(g.issuance_log
            .get(&registry_addr.to_lowercase())
            .cloned()
            .unwrap_or_default())
    }

    async fn verifier_capability_log(
        &self,
        registry_addr: &str,
        purpose: &str,
    ) -> Result<Vec<(String, bool)>, ChainError> {
        let g = self.inner.lock().unwrap();
        Self::provider_reads_fail(&g, registry_addr)?;
        Self::capability_log_reads_fail(&g, registry_addr)?;
        Ok(g.verifier_log
            .get(&(registry_addr.to_lowercase(), purpose.to_lowercase()))
            .cloned()
            .unwrap_or_default())
    }

    async fn approved_resolvers(
        &self,
        registry_addr: &str,
        kind: ResolverKind,
    ) -> Result<Vec<(String, bool)>, ChainError> {
        let g = self.inner.lock().unwrap();
        Self::provider_reads_fail(&g, registry_addr)?;
        Self::capability_log_reads_fail(&g, registry_addr)?;
        Ok(g.resolvers
            .get(&(registry_addr.to_lowercase(), kind.as_u8()))
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

pub fn grant_issuer_role_calldata(grantee: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IDogTagSBT::grantRoleCall {
        role: parse_b256(&issuer_role_key()),
        account: parse_addr(grantee),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

/// ABI-encoded `DogTagSBTConsent.revokeRole(ISSUER_ROLE, account)` — the withdrawal direction of
/// the mint-role card. A control that could only grant would imply a state it gives no way to
/// reach (the standing lever rule).
pub fn revoke_issuer_role_calldata(grantee: &str) -> String {
    use alloy::sol_types::SolCall;
    let call = IDogTagSBT::revokeRoleCall {
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

/// `ProviderRegistry.attachService(providerId, serviceAddress, generationId, expectedOwner)`.
///
/// `expectedOwner` is a transaction GUARD, never a selector: the contract compares it against the
/// `owner()` it read off the service and reverts `UnexpectedServiceOwner()` on a mismatch, then
/// stores the RESOLVED owner. So a wrong value can only refuse the send.
pub fn attach_service_calldata(
    provider_id: &str,
    service_address: &str,
    generation_id: &str,
    expected_owner: &str,
) -> String {
    use alloy::sol_types::SolCall;
    let call = IProviderRegistry::attachServiceCall {
        providerId: parse_b160(provider_id),
        serviceAddress: parse_addr(service_address),
        generationId: parse_b256(generation_id),
        expectedOwner: parse_addr(expected_owner),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

/// `ProviderRegistry.setServiceStanding(serviceAddress, newStanding)` calldata.
pub fn set_service_standing_calldata(service_address: &str, standing: Standing) -> String {
    use alloy::sol_types::SolCall;
    let call = IProviderRegistry::setServiceStandingCall {
        serviceAddress: parse_addr(service_address),
        newStanding: standing.as_u8(),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

/// `ProviderRegistry.setRights(account, rights)` calldata.
///
/// `rights` is the account's COMPLETE settable mask afterwards, not a delta — withdrawing every right
/// is `set_rights_calldata(account, 0)`.
pub fn set_rights_calldata(account: &str, rights: u64) -> String {
    use alloy::sol_types::SolCall;
    let call = IProviderRegistry::setRightsCall {
        account: parse_addr(account),
        rights: U256::from(rights),
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

/// `ProviderRegistry.setVerifierCapability(purpose, relayer, allowed)` calldata.
///
/// `purpose` is the RAW bytes32 from [`purpose_key`], NOT [`verify_key`]. The contract derives
/// `verificationKey(purpose)` itself, so handing it an already-derived key derives twice and writes
/// the capability under a key `canVerify` never reads - a write that succeeds and grants nothing.
pub fn set_verifier_capability_calldata(purpose: &str, relayer: &str, allowed: bool) -> String {
    use alloy::sol_types::SolCall;
    let call = IProviderRegistry::setVerifierCapabilityCall {
        purpose: parse_b256(purpose),
        relayer: parse_addr(relayer),
        allowed,
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

/// `ProviderRegistry.setResolverApproved(kind, resolver, approved)` calldata.
pub fn set_resolver_approved_calldata(
    kind: ResolverKind,
    resolver: &str,
    approved: bool,
) -> String {
    use alloy::sol_types::SolCall;
    let call = IProviderRegistry::setResolverApprovedCall {
        kind: kind.as_u8(),
        resolver: parse_addr(resolver),
        approved,
    };
    format!("0x{}", hex::encode(call.abi_encode()))
}

/// Parse a `bytes20` provider id. A malformed value yields zero, which every registrar write then
/// refuses on-chain with `ZeroProviderId()` - but the routes validate the shape first, so a
/// malformed id is a 400 rather than a wasted transaction.
fn parse_b160(h: &str) -> FixedBytes<20> {
    let s = h
        .strip_prefix("0x")
        .or_else(|| h.strip_prefix("0X"))
        .unwrap_or(h);
    let mut out = [0u8; 20];
    if let Ok(b) = hex::decode(s) {
        if b.len() == 20 {
            out.copy_from_slice(&b);
        }
    }
    FixedBytes::<20>::from(out)
}

/// The topic word for an indexed `bytes20 providerId`: LEFT-aligned, per Solidity's encoding of a
/// short fixed-bytes value. Right-aligning it (the address convention) matches no log at all, which
/// reads exactly like "this provider has never been approved for anything".
fn provider_id_topic(provider_id: &str) -> [u8; 32] {
    let mut topic = [0u8; 32];
    topic[..20].copy_from_slice(parse_b160(provider_id).as_slice());
    topic
}

/// The inverse: recover the `bytes20 providerId` from its topic word, taking the LEADING 20 bytes.
///
/// Named beside its forward twin rather than inlined at the call site because THIS is the direction
/// the whole-registry read depends on - that read passes no `providerId`, so it never builds a topic
/// and the only thing assigning a log to a provider is this decode. Reading `[12..]` instead (the
/// address convention) groups every log under a junk key, so every provider's approvals miss the
/// lookup and every row renders as approved for nothing - a definite false claim about the provider,
/// from a read that succeeded.
fn provider_id_from_topic(topic: &B256) -> String {
    format!("0x{}", hex::encode(&topic.as_slice()[..20]))
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
    /// Sending no `from` is correct for every read this is used for - they are all pure functions of
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

    async fn set_rights(
        &self,
        account_index: u32,
        registry_addr: &str,
        account: &str,
        rights: u64,
    ) -> Result<SentTx, ChainError> {
        let calldata = set_rights_calldata(account, rights);
        self.sign_and_send(account_index, registry_addr, &calldata)
            .await
    }

    async fn set_verifier_capability(
        &self,
        account_index: u32,
        registry_addr: &str,
        purpose: &str,
        relayer: &str,
        allowed: bool,
    ) -> Result<SentTx, ChainError> {
        let calldata = set_verifier_capability_calldata(purpose, relayer, allowed);
        self.sign_and_send(account_index, registry_addr, &calldata)
            .await
    }

    async fn is_whitelisted_for(
        &self,
        registry_addr: &str,
        record_type: &str,
        signer: &str,
    ) -> Result<bool, ChainError> {
        // A raw `eth_call` rather than a generated contract instance: `IProviderRegistry` carries no
        // `#[sol(rpc)]`, because that would generate a `provider()` accessor colliding with this
        // contract's own `provider(bytes20)`.
        use alloy::sol_types::SolCall;
        let call = IProviderRegistry::isWhitelistedForCall {
            key: parse_b256(record_type),
            signer: parse_addr(signer),
        };
        let out = self.raw_call(registry_addr, &call.abi_encode()).await?;
        Ok(
            IProviderRegistry::isWhitelistedForCall::abi_decode_returns(&out, true)
                .map_err(|e| ChainError::Other(format!("isWhitelistedFor decode: {e}")))?
                ._0,
        )
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

    async fn issuer_role_holders(&self, sbt_addr: &str) -> Result<Vec<String>, ChainError> {
        use alloy::providers::ProviderBuilder;
        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let c = IDogTagSBT::new(parse_addr(sbt_addr), provider);
        let role = parse_b256(&issuer_role_key());
        let count = c
            .getRoleMemberCount(role)
            .call()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?
            ._0;
        // Bounded defensively: the SBT grants this role to a handful of vet signers, so a count
        // beyond this reads as a wrong contract rather than a bigger fleet.
        let count = usize::try_from(count).unwrap_or(usize::MAX).min(256);
        let mut holders = Vec::with_capacity(count);
        for i in 0..count {
            let member = c
                .getRoleMember(role, U256::from(i))
                .call()
                .await
                .map_err(|e| ChainError::Rpc(e.to_string()))?
                ._0;
            holders.push(format!("{member:#x}"));
        }
        Ok(holders)
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
            // `provider()` does not revert for an unknown id, so a zero controller - the contract's
            // own existence sentinel (`_requireProvider`) - is what "not registered" means here.
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
        let by_provider = self
            .approval_log_ordered(registry_addr, Some(provider_id))
            .await?;
        Ok(by_provider
            .get(&provider_id.to_lowercase())
            .cloned()
            .unwrap_or_default())
    }

    async fn service_creation_approval_log_by_provider(
        &self,
        registry_addr: &str,
    ) -> Result<HashMap<String, Vec<(String, bool)>>, ChainError> {
        self.approval_log_ordered(registry_addr, None).await
    }

    async fn service_record(
        &self,
        registry_addr: &str,
        service_addr: &str,
    ) -> Result<ServiceRecord, ChainError> {
        use alloy::sol_types::SolCall;
        let call = IProviderRegistry::serviceCall {
            serviceAddress: parse_addr(service_addr),
        };
        let out = self.raw_call(registry_addr, &call.abi_encode()).await?;
        let s = IProviderRegistry::serviceCall::abi_decode_returns(&out, true)
            .map_err(|e| ChainError::Other(format!("service decode: {e}")))?
            ._0;
        let record_type_key = format!("0x{}", hex::encode(s.recordType.as_slice()));
        Ok(ServiceRecord {
            service_address: service_addr.to_lowercase(),
            provider_id: format!("0x{}", hex::encode(s.providerId.as_slice())),
            factory_generation: format!("0x{}", hex::encode(s.factoryGeneration.as_slice())),
            record_type: crate::provider_registry::record_type_label(&record_type_key),
            record_type_key,
            confirmed_owner: format!("{:#x}", s.confirmedOwner),
            domain_resolver: format!("{:#x}", s.domainResolver),
            owner_epoch: s.ownerEpoch,
            standing: Standing::from_u8(s.standing),
            // `service()` answers a zero-filled struct for an unknown address rather than reverting,
            // so `providerId != 0` - the contract's own `_requireService` sentinel - is existence.
            attached: !s.providerId.is_zero(),
        })
    }

    async fn provider_service_page(
        &self,
        registry_addr: &str,
        provider_id: &str,
        cursor: u64,
        limit: u64,
    ) -> Result<(Vec<String>, u64), ChainError> {
        use alloy::sol_types::SolCall;
        let call = IProviderRegistry::providerServicePageCall {
            providerId: parse_b160(provider_id),
            cursor: U256::from(cursor),
            limit: U256::from(limit),
        };
        let out = self.raw_call(registry_addr, &call.abi_encode()).await?;
        let r = IProviderRegistry::providerServicePageCall::abi_decode_returns(&out, true)
            .map_err(|e| ChainError::Other(format!("providerServicePage decode: {e}")))?;
        Ok((
            r.values.iter().map(|a| format!("{a:#x}")).collect(),
            r.nextCursor.to::<u64>(),
        ))
    }

    async fn service_effective(
        &self,
        registry_addr: &str,
        service_addr: &str,
    ) -> Result<ServiceEffective, ChainError> {
        use alloy::sol_types::SolCall;
        let call = IProviderRegistry::effectiveServiceCall {
            serviceAddress: parse_addr(service_addr),
        };
        let out = self.raw_call(registry_addr, &call.abi_encode()).await?;
        let r = IProviderRegistry::effectiveServiceCall::abi_decode_returns(&out, true)
            .map_err(|e| ChainError::Other(format!("effectiveService decode: {e}")))?;
        Ok(ServiceEffective {
            provider_standing: Standing::from_u8(r.providerStanding),
            service_standing: Standing::from_u8(r.serviceStanding),
            factory_active: r.factoryActive,
            owner_confirmed: r.ownerConfirmed,
            has_active_issuer: r.hasActiveIssuer,
        })
    }

    async fn current_service(
        &self,
        registry_addr: &str,
        provider_id: &str,
        record_type_key: &str,
    ) -> Result<String, ChainError> {
        use alloy::sol_types::SolCall;
        let call = IProviderRegistry::currentServiceCall {
            providerId: parse_b160(provider_id),
            recordType: parse_b256(record_type_key),
        };
        let out = self.raw_call(registry_addr, &call.abi_encode()).await?;
        let r = IProviderRegistry::currentServiceCall::abi_decode_returns(&out, true)
            .map_err(|e| ChainError::Other(format!("currentService decode: {e}")))?
            ._0;
        Ok(format!("{r:#x}"))
    }

    async fn factory_generations(
        &self,
        registry_addr: &str,
    ) -> Result<Vec<(String, String, bool)>, ChainError> {
        use alloy::sol_types::SolCall;
        let mut out = Vec::new();
        let mut cursor = 0u64;
        loop {
            let call = IProviderRegistry::factoryGenerationPageCall {
                cursor: U256::from(cursor),
                limit: U256::from(50u64),
            };
            let raw = self.raw_call(registry_addr, &call.abi_encode()).await?;
            let page = IProviderRegistry::factoryGenerationPageCall::abi_decode_returns(&raw, true)
                .map_err(|e| ChainError::Other(format!("factoryGenerationPage decode: {e}")))?;
            let next = page.nextCursor.to::<u64>();
            let empty = page.values.is_empty();
            for gid in page.values {
                let gid_hex = format!("0x{}", hex::encode(gid.as_slice()));
                let g_call = IProviderRegistry::factoryGenerationCall { generationId: gid };
                let g_raw = self.raw_call(registry_addr, &g_call.abi_encode()).await?;
                let g = IProviderRegistry::factoryGenerationCall::abi_decode_returns(&g_raw, true)
                    .map_err(|e| ChainError::Other(format!("factoryGeneration decode: {e}")))?
                    ._0;
                out.push((gid_hex, format!("{:#x}", g.factory), g.active));
            }
            if empty || next <= cursor {
                break;
            }
            cursor = next;
        }
        Ok(out)
    }

    async fn service_metadata(&self, service_addr: &str) -> Result<(String, String), ChainError> {
        use alloy::sol_types::SolCall;
        // Both reads must SUCCEED. A generation-1 `DogTagIssuer` implements `recordType()` but has
        // no `owner()` at all, so the owner read reverts - and reporting that as a zero address
        // would turn "this contract can never be attached" into a mismatch the admin might try to
        // correct by typing a different expected owner.
        let owner_out = self
            .raw_call(service_addr, &IServiceProbe::ownerCall {}.abi_encode())
            .await
            .map_err(|e| ChainError::Rpc(format!("service owner(): {e}")))?;
        let owner = IServiceProbe::ownerCall::abi_decode_returns(&owner_out, true)
            .map_err(|e| ChainError::Other(format!("owner decode: {e}")))?
            ._0;
        let rt_out = self
            .raw_call(service_addr, &IServiceProbe::recordTypeCall {}.abi_encode())
            .await
            .map_err(|e| ChainError::Rpc(format!("service recordType(): {e}")))?;
        let rt = IServiceProbe::recordTypeCall::abi_decode_returns(&rt_out, true)
            .map_err(|e| ChainError::Other(format!("recordType decode: {e}")))?
            ._0;
        Ok((
            format!("{owner:#x}"),
            format!("0x{}", hex::encode(rt.as_slice())),
        ))
    }

    async fn issuance_rights_log(
        &self,
        registry_addr: &str,
    ) -> Result<Vec<(String, bool)>, ChainError> {
        // `RightsSet` has ONE indexed arg (`account`, topic1) and one data word (`rights`), so this
        // cannot use `bool_log_ordered`: that helper reads a bool subject out of an indexed topic,
        // and here the subject IS topic1 while the value is a 256-bit mask in the body.
        self.rights_log_ordered(registry_addr).await
    }

    async fn verifier_capability_log(
        &self,
        registry_addr: &str,
        purpose: &str,
    ) -> Result<Vec<(String, bool)>, ChainError> {
        use alloy::sol_types::SolEvent;
        // THREE indexed args: topic1 = purpose (the group), topic2 = the derived compatibility key,
        // topic3 = relayer (the subject). Reading the subject off topic2 yields a bytes32 rendered
        // as an address - a plausible-looking value that is never a real relayer.
        self.bool_log_ordered(
            registry_addr,
            IProviderRegistry::VerifierCapabilitySet::SIGNATURE_HASH,
            Some(parse_b256(purpose)),
            3,
            "VerifierCapabilitySet",
        )
        .await
    }

    async fn approved_resolvers(
        &self,
        registry_addr: &str,
        kind: ResolverKind,
    ) -> Result<Vec<(String, bool)>, ChainError> {
        use alloy::sol_types::SolCall;
        let mut out = Vec::new();
        let mut cursor = 0u64;
        loop {
            let call = IProviderRegistry::resolverPageCall {
                kind: kind.as_u8(),
                cursor: U256::from(cursor),
                limit: U256::from(50u64),
            };
            let raw = self.raw_call(registry_addr, &call.abi_encode()).await?;
            let page = IProviderRegistry::resolverPageCall::abi_decode_returns(&raw, true)
                .map_err(|e| ChainError::Other(format!("resolverPage decode: {e}")))?;
            let next = page.nextCursor.to::<u64>();
            let empty = page.values.is_empty();
            for r in page.values {
                // The listing is append-only and RETAINS a deapproved resolver, so approval is read
                // per address rather than inferred from being listed: "approved then pulled" is a
                // different fact from "never approved", and only this flag separates them.
                let a_call = IProviderRegistry::isResolverApprovedCall {
                    kind: kind.as_u8(),
                    resolver: r,
                };
                let a_raw = self.raw_call(registry_addr, &a_call.abi_encode()).await?;
                let approved =
                    IProviderRegistry::isResolverApprovedCall::abi_decode_returns(&a_raw, true)
                        .map_err(|e| ChainError::Other(format!("isResolverApproved decode: {e}")))?
                        ._0;
                out.push((format!("{r:#x}"), approved));
            }
            if empty || next <= cursor {
                break;
            }
            cursor = next;
        }
        Ok(out)
    }
}

impl AlloyChain {
    /// The one `ServiceCreationApprovalSet` reader both approval methods share, so the topic
    /// alignment, the decode and the ORDERING rule below cannot drift into two versions.
    ///
    /// `provider_id` narrows the filter to one provider (the detail path); `None` reads the whole
    /// registry in a single scan (the list path).
    ///
    /// ## Order is established here, never assumed
    ///
    /// `fold_approvals` takes the LAST write per record type, so its ascending-`(block, logIndex)`
    /// precondition decides whether a withdrawal or the grant before it is reported as current. A
    /// node's `eth_getLogs` ordering is not part of that contract, so the pair is READ off each log
    /// and sorted here rather than trusted.
    ///
    /// A log carrying NEITHER position may be neither placed nor dropped - the same rule this repo
    /// applies to the issuer-whitelist pillar's `log_point`. Placing it at `(0, 0)` sorts it ahead of
    /// every real event, which can turn a withdrawal into an approval; dropping it discards the event
    /// that may itself have been the withdrawal. Both invent an answer, so an unpositioned log makes
    /// the whole read fail, which the caller renders as `Unavailable`.
    async fn approval_log_ordered(
        &self,
        registry_addr: &str,
        provider_id: Option<&str>,
    ) -> Result<HashMap<String, Vec<(String, bool)>>, ChainError> {
        use alloy::providers::{Provider, ProviderBuilder};
        use alloy::rpc::types::Filter;
        use alloy::sol_types::{SolEvent, SolValue};

        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let mut filter = Filter::new()
            .address(parse_addr(registry_addr))
            .event_signature(IProviderRegistry::ServiceCreationApprovalSet::SIGNATURE_HASH)
            .from_block(0u64);
        if let Some(id) = provider_id {
            // topic1 is the indexed `bytes20 providerId`. Solidity left-aligns a short fixed-bytes
            // value in its topic word, so the filter word is the id followed by 12 zero bytes -
            // right-aligning it (the address convention) matches no log at all, which would read
            // exactly like "this provider has never been approved for anything".
            filter = filter.topic1(B256::from(provider_id_topic(id)));
        }
        let logs = provider
            .get_logs(&filter)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        let mut entries = Vec::with_capacity(logs.len());
        for log in logs {
            // topic1 is the indexed `bytes20 providerId`, topic2 the indexed `bytes32 recordType`;
            // `allowed` is the sole unindexed member.
            let (Some(id_topic), Some(record_type)) = (log.topics().get(1), log.topics().get(2))
            else {
                continue;
            };
            let allowed = bool::abi_decode(log.data().data.as_ref(), true)
                .map_err(|e| ChainError::Other(format!("approval log decode: {e}")))?;
            entries.push(ApprovalLogEntry {
                block_number: log.block_number,
                log_index: log.log_index,
                provider_id: provider_id_from_topic(id_topic),
                record_type_key: format!("0x{}", hex::encode(record_type.as_slice())),
                allowed,
            });
        }
        order_approval_log(entries)
    }

    /// The ONE reader every other bool-valued registrar log goes through - issuance capability,
    /// verifier capability, and any future `(group, subject, bool)` event.
    ///
    /// Generalized rather than copied three times, because the part worth not duplicating is the
    /// ORDERING RULE: each of these is folded last-write-wins, so ordering decides whether a
    /// withdrawal or the grant before it is reported as current, and `eth_getLogs` ordering is not
    /// part of that contract. Three near-copies is three places for that rule to drift.
    ///
    /// `group_topic` filters on topic1; `subject_topic_index` says which topic carries the ADDRESS
    /// being granted (topic2 for a two-indexed event, topic3 where a derived key sits between).
    async fn bool_log_ordered(
        &self,
        registry_addr: &str,
        signature_hash: B256,
        group_topic: Option<B256>,
        subject_topic_index: usize,
        event_name: &str,
    ) -> Result<Vec<(String, bool)>, ChainError> {
        use alloy::providers::{Provider, ProviderBuilder};
        use alloy::rpc::types::Filter;
        use alloy::sol_types::SolValue;

        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let mut filter = Filter::new()
            .address(parse_addr(registry_addr))
            .event_signature(signature_hash)
            .from_block(0u64);
        if let Some(t) = group_topic {
            filter = filter.topic1(t);
        }
        let logs = provider
            .get_logs(&filter)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        let mut entries: Vec<(Option<u64>, Option<u64>, String, bool)> =
            Vec::with_capacity(logs.len());
        for log in logs {
            let Some(subject) = log.topics().get(subject_topic_index) else {
                continue;
            };
            let allowed = bool::abi_decode(log.data().data.as_ref(), true)
                .map_err(|e| ChainError::Other(format!("{event_name} decode: {e}")))?;
            // An address topic is right-aligned: the low 20 bytes are the address.
            let addr = format!("0x{}", hex::encode(&subject.as_slice()[12..32]));
            entries.push((log.block_number, log.log_index, addr, allowed));
        }

        // Same rule as `order_approval_log`, and for the same reason: a log carrying NEITHER
        // position may be neither placed nor dropped. Placing it at `(0, 0)` sorts it ahead of every
        // real event, which can present a superseded grant as current; dropping it discards the
        // event that may itself have been the withdrawal. Both invent an answer, so the whole read
        // fails and the caller renders `Unavailable`.
        if entries
            .iter()
            .any(|(b, i, _, _)| b.is_none() || i.is_none())
        {
            return Err(ChainError::Other(format!(
                "a {event_name} log carried no (blockNumber, logIndex), so the capability could not \
                 be ordered - the last write per holder is what makes this the current state, and an \
                 unpositioned log can be neither placed nor dropped without inventing an answer"
            )));
        }
        entries.sort_by_key(|(b, i, _, _)| (*b, *i));
        Ok(entries
            .into_iter()
            .map(|(_, _, addr, allowed)| (addr, allowed))
            .collect())
    }

    /// The `RightsSet` log for a whole registry, folded to `(account, holds_issue)` in log order.
    ///
    /// Deliberately not expressed through [`Self::bool_log_ordered`]: that helper reads the SUBJECT
    /// out of an indexed topic and the VALUE out of a bool body, and `RightsSet` has the subject in
    /// topic1 and a 256-bit mask in the body. Forcing it through would need the subject index to mean
    /// something different for this one event, which is how a decoder comes to read the wrong word.
    async fn rights_log_ordered(
        &self,
        registry_addr: &str,
    ) -> Result<Vec<(String, bool)>, ChainError> {
        use alloy::providers::{Provider, ProviderBuilder};
        use alloy::rpc::types::Filter;
        use alloy::sol_types::{SolEvent, SolValue};

        let provider = ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;
        let logs = provider
            .get_logs(
                &Filter::new()
                    .address(parse_addr(registry_addr))
                    .event_signature(IProviderRegistry::RightsSet::SIGNATURE_HASH)
                    .from_block(0u64),
            )
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        let settable = U256::from(dogtag_standard::verify::RIGHT_ISSUE);
        let mut entries: Vec<(Option<u64>, Option<u64>, String, bool)> =
            Vec::with_capacity(logs.len());
        for log in logs {
            // topic1 is the indexed `account`. An address topic is RIGHT-aligned, so the low 20
            // bytes are the address.
            let Some(subject) = log.topics().get(1) else {
                continue;
            };
            let rights = U256::abi_decode(log.data().data.as_ref(), true)
                .map_err(|e| ChainError::Other(format!("RightsSet decode: {e}")))?;
            let addr = format!("0x{}", hex::encode(&subject.as_slice()[12..32]));
            // Masked in the FULL 256-bit width — never truncated to a u64 first.
            entries.push((
                log.block_number,
                log.log_index,
                addr,
                rights & settable != U256::ZERO,
            ));
        }

        // Same rule as `bool_log_ordered`: an unpositioned log may be neither placed nor dropped.
        if entries
            .iter()
            .any(|(b, i, _, _)| b.is_none() || i.is_none())
        {
            return Err(ChainError::Other(
                "a RightsSet log carried no (blockNumber, logIndex), so the rights could not be \
                 ordered - the last write per holder is what makes this the current state, and an \
                 unpositioned log can be neither placed nor dropped without inventing an answer"
                    .into(),
            ));
        }
        entries.sort_by_key(|(b, i, _, _)| (*b, *i));
        Ok(entries
            .into_iter()
            .map(|(_, _, addr, allowed)| (addr, allowed))
            .collect())
    }
}

/// One decoded `ServiceCreationApprovalSet` log, before ordering.
struct ApprovalLogEntry {
    block_number: Option<u64>,
    log_index: Option<u64>,
    provider_id: String,
    record_type_key: String,
    allowed: bool,
}

/// Sort a decoded approval log into ascending `(blockNumber, logIndex)` and group it by provider.
///
/// Extracted from the `eth_getLogs` call so it is reachable by a test: this rule lives on the Alloy
/// path alone and `MemChain` is a different `ChainClient`, so nothing driving the fake can pin it -
/// the same reason the issuer-whitelist pillar's error classifier is a free function.
///
/// An entry missing EITHER position fails the whole read. Placing it at `(0, 0)` would sort it ahead
/// of every real event, which can present a superseded grant as current; dropping it would discard
/// the event that may itself have been the withdrawal. Both fabricate an answer, and this repo's
/// standing rule is that a positionless log is neither placed nor dropped.
fn order_approval_log(
    mut entries: Vec<ApprovalLogEntry>,
) -> Result<HashMap<String, Vec<(String, bool)>>, ChainError> {
    if entries
        .iter()
        .any(|e| e.block_number.is_none() || e.log_index.is_none())
    {
        return Err(ChainError::Other(
            "a ServiceCreationApprovalSet log carried no (blockNumber, logIndex), so the approvals \
             could not be ordered - the last write per record type is what makes this the current \
             state, and an unpositioned log can be neither placed nor dropped without inventing an \
             answer"
                .into(),
        ));
    }
    // Sorting rather than trusting the node: `fold_approvals` takes the LAST write per record type,
    // so ordering decides whether a withdrawal or the grant before it is reported as current, and
    // `eth_getLogs` ordering is not part of that contract.
    entries.sort_by_key(|e| (e.block_number, e.log_index));

    let mut out: HashMap<String, Vec<(String, bool)>> = HashMap::new();
    for e in entries {
        out.entry(e.provider_id.to_ascii_lowercase())
            .or_default()
            .push((e.record_type_key, e.allowed));
    }
    Ok(out)
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
    /// The fake refuses exactly what the contract refuses.
    ///
    /// `RIGHT_ISSUE` is the ONLY settable bit today, so "the mask equals 1" and "bit 0 is set" agree
    /// on every value the chain accepts - which makes a whole-word comparison in `set_rights` a
    /// BEHAVIOUR-PRESERVING change right now rather than an unpinned claim. What is pinnable, and
    /// what actually keeps the fake honest, is that an unsettable bit is REFUSED here exactly as
    /// `setRights` refuses it with `UnsettableRights()` - so a route that ever sends a compound mask
    /// fails in tests instead of reverting on ROAX.
    #[tokio::test]
    async fn set_rights_refuses_a_bit_the_contract_would_refuse() {
        let c = MemChain::new();
        c.register_signer(0, [0u8; 32], "0xabc".into()).await;
        let reg = "0x00000000000000000000000000000000000000aa";
        let who = "0x00000000000000000000000000000000000000bb";

        // The settable grant lands.
        assert!(c
            .set_rights(0, reg, who, dogtag_standard::verify::RIGHT_ISSUE)
            .await
            .is_ok());

        // A derived bit, and a bit at an unallocated position, are both refused.
        for bad in [
            1u64 << 1,
            1 << 5,
            1 << 6,
            dogtag_standard::verify::RIGHT_ISSUE | (1 << 1),
        ] {
            let e = c.set_rights(0, reg, who, bad).await.unwrap_err();
            assert!(
                format!("{e}").contains("UnsettableRights"),
                "mask {bad:#x} must be refused as unsettable, got: {e}"
            );
        }
    }

    use super::*;

    const RT_A: &str = "0x00000000000000000000000000000000000000000000000000000000000000aa";
    const RT_B: &str = "0x00000000000000000000000000000000000000000000000000000000000000bb";
    const PROVIDER_A: &str = "0x00000000000000000000000000000000000000a1";
    const PROVIDER_B: &str = "0x00000000000000000000000000000000000000b2";

    fn approval_entry(
        block: Option<u64>,
        index: Option<u64>,
        provider_id: &str,
        record_type_key: &str,
        allowed: bool,
    ) -> ApprovalLogEntry {
        ApprovalLogEntry {
            block_number: block,
            log_index: index,
            provider_id: provider_id.to_string(),
            record_type_key: record_type_key.to_string(),
            allowed,
        }
    }

    /// The read establishes order rather than trusting the node's. `fold_approvals` takes the LAST
    /// write per record type, so a peer returning the withdrawal before the grant it superseded
    /// would otherwise render a withdrawn record type as approved - and the route's `NoChange`
    /// pre-check would then refuse the corrective write.
    #[test]
    fn approval_logs_are_ordered_by_block_and_log_index_not_by_arrival() {
        let out = order_approval_log(vec![
            approval_entry(Some(9), Some(0), PROVIDER_A, RT_A, false),
            approval_entry(Some(2), Some(1), PROVIDER_A, RT_A, true),
        ])
        .expect("both logs carry a position");
        assert_eq!(
            out.get(PROVIDER_A).unwrap(),
            &vec![(RT_A.to_string(), true), (RT_A.to_string(), false)],
            "the earlier grant must fold first, so the later withdrawal wins"
        );
        // Same block, so only `logIndex` can sequence the pair.
        let out = order_approval_log(vec![
            approval_entry(Some(7), Some(3), PROVIDER_A, RT_A, false),
            approval_entry(Some(7), Some(1), PROVIDER_A, RT_A, true),
        ])
        .expect("both logs carry a position");
        assert!(
            !out.get(PROVIDER_A).unwrap().last().unwrap().1,
            "logIndex must break a same-block tie"
        );
    }

    /// A log carrying no position may be NEITHER placed nor dropped: `(0, 0)` sorts it ahead of every
    /// real event (turning a superseded grant into the current state) and dropping it discards the
    /// event that may itself have been the withdrawal. The whole read fails, which the route renders
    /// as `Unavailable` rather than as an approval answer.
    #[test]
    fn an_unpositioned_approval_log_fails_the_whole_read() {
        for entry in [
            approval_entry(None, Some(0), PROVIDER_A, RT_A, true),
            approval_entry(Some(4), None, PROVIDER_A, RT_A, true),
        ] {
            let out = order_approval_log(vec![
                approval_entry(Some(1), Some(0), PROVIDER_A, RT_A, false),
                entry,
            ]);
            assert!(
                out.is_err(),
                "a positionless log must fail the read, never be silently placed or dropped"
            );
        }
    }

    /// The whole-registry read groups by provider, so one scan can serve a page without leaking one
    /// provider's approvals into another's row.
    #[test]
    fn the_registry_wide_read_groups_by_provider() {
        let out = order_approval_log(vec![
            approval_entry(Some(1), Some(0), PROVIDER_A, RT_A, true),
            approval_entry(Some(2), Some(0), PROVIDER_B, RT_B, true),
            approval_entry(Some(3), Some(0), PROVIDER_A, RT_B, false),
        ])
        .expect("every log carries a position");
        assert_eq!(out.len(), 2);
        assert_eq!(
            out.get(PROVIDER_A).unwrap(),
            &vec![(RT_A.to_string(), true), (RT_B.to_string(), false)]
        );
        assert_eq!(
            out.get(PROVIDER_B).unwrap(),
            &vec![(RT_B.to_string(), true)]
        );
    }

    /// The indexed `bytes20 providerId` is LEFT-aligned in its topic word. Right-aligning it (the
    /// address convention) matches no log at all, which reads exactly like "this provider has never
    /// been approved for anything".
    #[test]
    fn the_provider_id_topic_is_left_aligned() {
        let topic = provider_id_topic(PROVIDER_A);
        assert_eq!(
            format!("0x{}", hex::encode(&topic[..20])),
            PROVIDER_A,
            "the id must occupy the LEADING 20 bytes of the topic word"
        );
        assert!(
            topic[20..].iter().all(|b| *b == 0),
            "the trailing 12 bytes must be the padding"
        );
    }

    /// The INVERSE, which is the direction the whole-registry read actually depends on: that read
    /// passes no `providerId`, so it builds no topic and this decode is the only thing assigning a
    /// log to a provider.
    ///
    /// Asserted against a hand-built topic word rather than only round-tripped, because a round trip
    /// alone is satisfied by BOTH directions being wrong the same way - encode to `[12..]` and decode
    /// from `[12..]` and the pair still agrees, while every real log decodes to a junk key.
    #[test]
    fn a_provider_id_is_recovered_from_the_leading_bytes_of_its_topic_word() {
        let id_bytes = hex::decode(PROVIDER_A.trim_start_matches("0x")).unwrap();

        let mut left_aligned = [0u8; 32];
        left_aligned[..20].copy_from_slice(&id_bytes);
        assert_eq!(
            provider_id_from_topic(&B256::from(left_aligned)),
            PROVIDER_A,
            "a Solidity-encoded bytes20 topic carries the id in its LEADING 20 bytes"
        );

        // The address convention, and the exact mistake the surrounding comments warn about: it must
        // not recover the id, or nothing distinguishes the two encodings.
        let mut right_aligned = [0u8; 32];
        right_aligned[12..].copy_from_slice(&id_bytes);
        assert_ne!(
            provider_id_from_topic(&B256::from(right_aligned)),
            PROVIDER_A,
            "reading the TRAILING 20 bytes is the address convention, not the bytes20 encoding"
        );

        // And the pair agrees, so the filter the detail path builds and the decode the list path
        // performs cannot drift apart.
        assert_eq!(
            provider_id_from_topic(&B256::from(provider_id_topic(PROVIDER_A))),
            PROVIDER_A
        );
    }

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

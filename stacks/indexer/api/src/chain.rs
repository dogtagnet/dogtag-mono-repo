//! `LogSource` — the ROAX (chainId 135) read surface the indexer scans: `eth_getLogs` over the
//! factory / registry / VerificationRegistry / all `DogTagIssuer` clones, plus block-head + block-hash
//! reads for the resume cursor and reorg detection.
//!
//! `AlloyLogSource` is the real RPC scanner (mirrors the alloy surface the government/vet stacks use);
//! `MemLogSource` is an in-memory scriptable source so the ingest loop + query API are testable
//! without a live node (the same MemChain pattern the sibling stacks use).
//!
//! Anti-spoof: `eth_getLogs` is filtered by event *signature* (topic0) with no address filter, so it
//! catches every clone's `RootIssued`/`RootRevoked` regardless of when the clone was deployed. Each
//! decoded log is then gated by its emitting address — factory events must come from the factory, a
//! clone event must come from a *known* clone (seeded from deployment config + `IssuerCreated`
//! discovery) — so a random contract re-emitting the same signature is dropped, not indexed.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use alloy::primitives::{Address, B256};
use alloy::sol;
use alloy::sol_types::SolEvent;
use async_trait::async_trait;

use crate::events::{EventType, Finality, IndexedEvent};

pub const ROAX_CHAIN_ID: u64 = 135;

sol! {
    #[sol(rpc)]
    contract IDogTagIssuerFactory {
        event IssuerCreated(address indexed clone, bytes32 indexed recordType, string name);
        event RootRegistered(bytes32 indexed root, address indexed clone);
    }
    #[sol(rpc)]
    contract IIssuerRegistry {
        event Whitelisted(bytes32 indexed recordType, address indexed signer);
        event Delisted(bytes32 indexed recordType, address indexed signer);
    }
    #[sol(rpc)]
    contract IDogTagIssuer {
        event RootIssued(bytes32 indexed root, address indexed by, uint256 ts);
        event RootRevoked(bytes32 indexed root, address indexed by, uint256 ts);
    }
    #[sol(rpc)]
    contract IVerificationRegistry {
        event Verified(
            uint256 indexed dogTagId,
            address indexed relayer,
            address indexed subject,
            bytes32 purpose,
            bytes32 nullifier,
            uint256 ts
        );
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChainError {
    #[error("rpc: {0}")]
    Rpc(String),
    #[error("{0}")]
    Other(String),
}

/// The fixed contract addresses + the current known-clone set a scan needs to gate events. Passed by
/// the ingest loop into each `fetch_events` call. Addresses are lowercase `0x…`.
#[derive(Clone, Debug)]
pub struct WatchContext {
    pub factory: String,
    pub registry: String,
    pub verification_registry: String,
    /// The clones known *before* this range (deployment seed + earlier `IssuerCreated`). Clones
    /// discovered *within* the range are folded in during decode (an `IssuerCreated` always precedes
    /// that clone's first `RootIssued` in `(block, logIndex)` order), so same-range issuance is caught.
    pub known_clones: HashSet<String>,
}

/// The set of topic0 signature hashes the scanner filters on.
pub fn watched_topic0() -> Vec<B256> {
    vec![
        IDogTagIssuerFactory::IssuerCreated::SIGNATURE_HASH,
        IDogTagIssuerFactory::RootRegistered::SIGNATURE_HASH,
        IIssuerRegistry::Whitelisted::SIGNATURE_HASH,
        IIssuerRegistry::Delisted::SIGNATURE_HASH,
        IDogTagIssuer::RootIssued::SIGNATURE_HASH,
        IDogTagIssuer::RootRevoked::SIGNATURE_HASH,
        IVerificationRegistry::Verified::SIGNATURE_HASH,
    ]
}

/// Abstract log surface. Block ranges are inclusive.
#[async_trait]
pub trait LogSource: Send + Sync {
    fn chain_id(&self) -> u64 {
        ROAX_CHAIN_ID
    }
    /// The current chain head (`eth_blockNumber`).
    async fn head_block(&self) -> Result<u64, ChainError>;
    /// The current **finalized** block height via the `finalized` block tag (ROAX/PoS finality).
    /// `Ok(Some(n))` when the node exposes the tag; `Ok(None)` when it does not (or has no finalized
    /// block yet), in which case the ingest loop falls back to a confirmations-depth watermark.
    async fn finalized_block(&self) -> Result<Option<u64>, ChainError>;
    /// Fetch + decode + timestamp every watched event in `[from, to]`, gated by `ctx`. Returns them
    /// sorted `(block, logIndex)` ascending so the caller can fold `IssuerCreated` discoveries in order.
    async fn fetch_events(
        &self,
        from: u64,
        to: u64,
        ctx: &WatchContext,
    ) -> Result<Vec<IndexedEvent>, ChainError>;
    /// The live hash of `block` (`0x…`), for reorg detection. `None` if the block is unknown.
    async fn block_hash(&self, block: u64) -> Result<Option<String>, ChainError>;
}

// ------------------------------------------------------------------------------------------------
// Decode: a raw log tuple -> IndexedEvent. Shared by both sources (Alloy decodes real logs into this
// tuple; MemLogSource constructs the tuple directly).
// ------------------------------------------------------------------------------------------------

/// A minimal, source-agnostic view of one log the decoder needs.
pub struct RawLog {
    pub address: String,
    pub topics: Vec<B256>,
    pub data: Vec<u8>,
    pub block_number: u64,
    pub block_hash: String,
    pub tx_hash: String,
    pub log_index: u64,
    pub block_timestamp: u64,
}

fn hexb(b: &B256) -> String {
    format!("0x{}", hex::encode(b.as_slice()))
}
fn hexa(a: &Address) -> String {
    format!("{a:#x}")
}

/// Decode one raw log into an `IndexedEvent`, gating by emitting address. Returns `None` for a log
/// that does not match a watched signature or that fails the anti-spoof address gate. `known` is the
/// mutable known-clone set (an `IssuerCreated` inserts into it so a same-range `RootIssued` passes).
pub fn decode_log(log: &RawLog, ctx: &WatchContext, known: &mut HashSet<String>) -> Option<IndexedEvent> {
    let topic0 = *log.topics.first()?;
    let addr = log.address.to_ascii_lowercase();
    let base = |event_type: EventType| IndexedEvent {
        id: IndexedEvent::make_id(&log.tx_hash, log.log_index),
        event_type,
        contract: addr.clone(),
        block_number: log.block_number,
        block_hash: log.block_hash.clone(),
        tx_hash: log.tx_hash.to_ascii_lowercase(),
        log_index: log.log_index,
        block_timestamp: log.block_timestamp,
        // Default pending; the ingest loop stamps the true finality from the finalized watermark.
        finality: Finality::Pending,
        actor: None,
        clone: None,
        record_type: None,
        name: None,
        root: None,
        dog_tag_id: None,
        subject: None,
        purpose: None,
        nullifier: None,
        onchain_ts: None,
    };
    // alloy's SolEvent::decode_raw_log validates topic0 + arity for us.
    if topic0 == IDogTagIssuerFactory::IssuerCreated::SIGNATURE_HASH {
        if addr != ctx.factory {
            return None; // spoof: only the real factory emits IssuerCreated
        }
        let d = IDogTagIssuerFactory::IssuerCreated::decode_raw_log(log.topics.iter().copied(), &log.data, true).ok()?;
        let clone = hexa(&d.clone).to_ascii_lowercase();
        known.insert(clone.clone());
        let mut e = base(EventType::IssuerCreated);
        e.clone = Some(clone);
        e.record_type = Some(hexb(&d.recordType));
        e.name = Some(d.name);
        Some(e)
    } else if topic0 == IDogTagIssuerFactory::RootRegistered::SIGNATURE_HASH {
        if addr != ctx.factory {
            return None;
        }
        let d = IDogTagIssuerFactory::RootRegistered::decode_raw_log(log.topics.iter().copied(), &log.data, true).ok()?;
        let mut e = base(EventType::RootRegistered);
        e.root = Some(hexb(&d.root));
        e.clone = Some(hexa(&d.clone).to_ascii_lowercase());
        Some(e)
    } else if topic0 == IIssuerRegistry::Whitelisted::SIGNATURE_HASH {
        if addr != ctx.registry {
            return None;
        }
        let d = IIssuerRegistry::Whitelisted::decode_raw_log(log.topics.iter().copied(), &log.data, true).ok()?;
        let mut e = base(EventType::Whitelisted);
        e.record_type = Some(hexb(&d.recordType));
        e.actor = Some(hexa(&d.signer).to_ascii_lowercase());
        Some(e)
    } else if topic0 == IIssuerRegistry::Delisted::SIGNATURE_HASH {
        if addr != ctx.registry {
            return None;
        }
        let d = IIssuerRegistry::Delisted::decode_raw_log(log.topics.iter().copied(), &log.data, true).ok()?;
        let mut e = base(EventType::Delisted);
        e.record_type = Some(hexb(&d.recordType));
        e.actor = Some(hexa(&d.signer).to_ascii_lowercase());
        Some(e)
    } else if topic0 == IDogTagIssuer::RootIssued::SIGNATURE_HASH {
        if !known.contains(&addr) {
            return None; // spoof: RootIssued must come from a known DogTagIssuer clone
        }
        let d = IDogTagIssuer::RootIssued::decode_raw_log(log.topics.iter().copied(), &log.data, true).ok()?;
        let mut e = base(EventType::RootIssued);
        e.root = Some(hexb(&d.root));
        e.actor = Some(hexa(&d.by).to_ascii_lowercase());
        e.clone = Some(addr);
        e.onchain_ts = Some(d.ts.to::<u64>());
        Some(e)
    } else if topic0 == IDogTagIssuer::RootRevoked::SIGNATURE_HASH {
        if !known.contains(&addr) {
            return None;
        }
        let d = IDogTagIssuer::RootRevoked::decode_raw_log(log.topics.iter().copied(), &log.data, true).ok()?;
        let mut e = base(EventType::RootRevoked);
        e.root = Some(hexb(&d.root));
        e.actor = Some(hexa(&d.by).to_ascii_lowercase());
        e.clone = Some(addr);
        e.onchain_ts = Some(d.ts.to::<u64>());
        Some(e)
    } else if topic0 == IVerificationRegistry::Verified::SIGNATURE_HASH {
        if addr != ctx.verification_registry {
            return None;
        }
        let d = IVerificationRegistry::Verified::decode_raw_log(log.topics.iter().copied(), &log.data, true).ok()?;
        let mut e = base(EventType::Verified);
        e.dog_tag_id = Some(d.dogTagId.to_string());
        e.actor = Some(hexa(&d.relayer).to_ascii_lowercase());
        e.subject = Some(hexa(&d.subject).to_ascii_lowercase());
        e.purpose = Some(hexb(&d.purpose));
        e.nullifier = Some(hexb(&d.nullifier));
        e.onchain_ts = Some(d.ts.to::<u64>());
        Some(e)
    } else {
        None
    }
}

// ------------------------------------------------------------------------------------------------
// AlloyLogSource — real ROAX RPC.
// ------------------------------------------------------------------------------------------------

pub struct AlloyLogSource {
    rpc_url: String,
    chain_id: u64,
    /// block number -> (timestamp, hash) cache, so re-scanned/overlapping ranges don't re-fetch headers.
    block_cache: Mutex<HashMap<u64, (u64, String)>>,
}

impl AlloyLogSource {
    pub fn new(rpc_url: impl Into<String>) -> Self {
        AlloyLogSource {
            rpc_url: rpc_url.into(),
            chain_id: ROAX_CHAIN_ID,
            block_cache: Mutex::new(HashMap::new()),
        }
    }
    pub fn with_chain_id(mut self, chain_id: u64) -> Self {
        self.chain_id = chain_id;
        self
    }

    async fn provider(
        &self,
    ) -> Result<impl alloy::providers::Provider, ChainError> {
        use alloy::providers::ProviderBuilder;
        ProviderBuilder::new()
            .on_builtin(&self.rpc_url)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))
    }

    /// Resolve (timestamp, hash) for a block, memoized.
    async fn block_meta(&self, n: u64) -> Result<(u64, String), ChainError> {
        if let Some(m) = self.block_cache.lock().unwrap().get(&n) {
            return Ok(m.clone());
        }
        use alloy::eips::BlockNumberOrTag;
        use alloy::network::primitives::BlockTransactionsKind;
        use alloy::providers::Provider;
        let provider = self.provider().await?;
        let block = provider
            .get_block_by_number(BlockNumberOrTag::Number(n), BlockTransactionsKind::Hashes)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?
            .ok_or_else(|| ChainError::Other(format!("block {n} not found")))?;
        let ts = block.header.timestamp;
        let hash = format!("{:#x}", block.header.hash);
        self.block_cache.lock().unwrap().insert(n, (ts, hash.clone()));
        Ok((ts, hash))
    }
}

#[async_trait]
impl LogSource for AlloyLogSource {
    fn chain_id(&self) -> u64 {
        self.chain_id
    }

    async fn head_block(&self) -> Result<u64, ChainError> {
        use alloy::providers::Provider;
        let provider = self.provider().await?;
        provider
            .get_block_number()
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))
    }

    async fn finalized_block(&self) -> Result<Option<u64>, ChainError> {
        use alloy::eips::BlockNumberOrTag;
        use alloy::network::primitives::BlockTransactionsKind;
        use alloy::providers::Provider;
        let provider = self.provider().await?;
        // A node without the finality gadget returns null (or errors) for the `finalized` tag; treat
        // either as "no finality tag" so the ingest loop falls back to a confirmations-depth watermark.
        match provider
            .get_block_by_number(BlockNumberOrTag::Finalized, BlockTransactionsKind::Hashes)
            .await
        {
            Ok(Some(block)) => Ok(Some(block.header.number)),
            Ok(None) => Ok(None),
            Err(e) => {
                tracing::debug!("finalized block tag unavailable ({e}); using confirmations fallback");
                Ok(None)
            }
        }
    }

    async fn fetch_events(
        &self,
        from: u64,
        to: u64,
        ctx: &WatchContext,
    ) -> Result<Vec<IndexedEvent>, ChainError> {
        use alloy::providers::Provider;
        use alloy::rpc::types::Filter;
        let provider = self.provider().await?;
        let filter = Filter::new()
            .from_block(from)
            .to_block(to)
            .event_signature(watched_topic0());
        let logs = provider
            .get_logs(&filter)
            .await
            .map_err(|e| ChainError::Rpc(e.to_string()))?;

        // Sort ascending by (block, logIndex) so IssuerCreated is folded into `known` before the
        // clone's RootIssued in the same range.
        let mut raw: Vec<(u64, u64, alloy::rpc::types::Log)> = logs
            .into_iter()
            .map(|l| (l.block_number.unwrap_or(0), l.log_index.unwrap_or(0), l))
            .collect();
        raw.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));

        let mut known = ctx.known_clones.clone();
        let mut out = Vec::new();
        for (bn, li, l) in raw {
            let (ts, bhash) = self.block_meta(bn).await?;
            let rl = RawLog {
                address: format!("{:#x}", l.inner.address),
                topics: l.inner.data.topics().to_vec(),
                data: l.inner.data.data.to_vec(),
                block_number: bn,
                block_hash: l
                    .block_hash
                    .map(|h| format!("{h:#x}"))
                    .unwrap_or(bhash),
                tx_hash: l
                    .transaction_hash
                    .map(|h| format!("{h:#x}"))
                    .unwrap_or_default(),
                log_index: li,
                block_timestamp: ts,
            };
            if let Some(ev) = decode_log(&rl, ctx, &mut known) {
                out.push(ev);
            }
        }
        Ok(out)
    }

    async fn block_hash(&self, block: u64) -> Result<Option<String>, ChainError> {
        match self.block_meta(block).await {
            Ok((_, hash)) => Ok(Some(hash)),
            Err(ChainError::Other(_)) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

// ------------------------------------------------------------------------------------------------
// MemLogSource — scriptable in-memory source for demo/local + tests.
// ------------------------------------------------------------------------------------------------

/// A scripted block: its hash + the raw logs it carries. Tests push these to drive the ingest loop
/// with no node. `reorg` swaps blocks at/after a height to a new hash to exercise rollback.
#[derive(Clone)]
struct MemBlock {
    hash: String,
    timestamp: u64,
    logs: Vec<MemRawLog>,
}

#[derive(Clone)]
struct MemRawLog {
    address: String,
    topics: Vec<B256>,
    data: Vec<u8>,
    tx_hash: String,
    log_index: u64,
}

#[derive(Clone, Default)]
pub struct MemLogSource {
    inner: Arc<Mutex<Vec<MemBlock>>>, // index == block number
    /// Optional finalized-height override. `None` ⇒ the source reports no finality tag (the ingest
    /// loop falls back to a confirmations-depth watermark), exactly like a node without the gadget.
    finalized: Arc<Mutex<Option<u64>>>,
}

impl MemLogSource {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the finalized-block height the source reports (simulates the `finalized` tag). Blocks at or
    /// below this height are treated as immutable by the ingest loop.
    pub fn set_finalized(&self, height: u64) {
        *self.finalized.lock().unwrap() = Some(height);
    }

    /// Append a block carrying pre-encoded raw logs. Returns the assigned block number.
    fn push_block(&self, hash: &str, timestamp: u64, logs: Vec<MemRawLog>) -> u64 {
        let mut g = self.inner.lock().unwrap();
        g.push(MemBlock {
            hash: hash.to_string(),
            timestamp,
            logs,
        });
        (g.len() - 1) as u64
    }

    /// Convenience: append an empty block (advances the head with no events).
    pub fn push_empty_block(&self, hash: &str, timestamp: u64) -> u64 {
        self.push_block(hash, timestamp, vec![])
    }

    /// Append a block carrying a set of alloy-encoded events. Each `(address, event_signature_hash,
    /// topics, data)` tuple is built by the `emit_*` helpers below. `tx_hash` is derived from the
    /// block hash + index so ids are stable.
    pub fn push_events(&self, hash: &str, timestamp: u64, evs: Vec<EncodedLog>) -> u64 {
        let logs = evs
            .into_iter()
            .enumerate()
            .map(|(i, e)| MemRawLog {
                address: e.address,
                topics: e.topics,
                data: e.data,
                tx_hash: format!("{hash}{i:02x}"),
                log_index: i as u64,
            })
            .collect();
        self.push_block(hash, timestamp, logs)
    }

    /// Rewrite the chain at/after `block` with a fresh set of blocks (simulates a reorg). Blocks
    /// before `block` are retained.
    pub fn reorg_from(&self, block: u64, new_blocks: Vec<(String, u64, Vec<EncodedLog>)>) {
        let mut g = self.inner.lock().unwrap();
        g.truncate(block as usize);
        drop(g);
        for (hash, ts, evs) in new_blocks {
            self.push_events(&hash, ts, evs);
        }
    }
}

/// An alloy-encoded log ready to be scripted into a `MemLogSource` block.
#[derive(Clone)]
pub struct EncodedLog {
    pub address: String,
    pub topics: Vec<B256>,
    pub data: Vec<u8>,
}

#[async_trait]
impl LogSource for MemLogSource {
    async fn head_block(&self) -> Result<u64, ChainError> {
        let g = self.inner.lock().unwrap();
        if g.is_empty() {
            return Ok(0);
        }
        Ok((g.len() - 1) as u64)
    }

    async fn finalized_block(&self) -> Result<Option<u64>, ChainError> {
        Ok(*self.finalized.lock().unwrap())
    }

    async fn fetch_events(
        &self,
        from: u64,
        to: u64,
        ctx: &WatchContext,
    ) -> Result<Vec<IndexedEvent>, ChainError> {
        let g = self.inner.lock().unwrap();
        let mut known = ctx.known_clones.clone();
        let mut out = Vec::new();
        for bn in from..=to {
            let Some(block) = g.get(bn as usize) else {
                continue;
            };
            for l in &block.logs {
                let rl = RawLog {
                    address: l.address.clone(),
                    topics: l.topics.clone(),
                    data: l.data.clone(),
                    block_number: bn,
                    block_hash: block.hash.clone(),
                    tx_hash: l.tx_hash.clone(),
                    log_index: l.log_index,
                    block_timestamp: block.timestamp,
                };
                if let Some(ev) = decode_log(&rl, ctx, &mut known) {
                    out.push(ev);
                }
            }
        }
        Ok(out)
    }

    async fn block_hash(&self, block: u64) -> Result<Option<String>, ChainError> {
        let g = self.inner.lock().unwrap();
        Ok(g.get(block as usize).map(|b| b.hash.clone()))
    }
}

// ------------------------------------------------------------------------------------------------
// Test helpers: build alloy-encoded logs for MemLogSource. Kept in-crate (not #[cfg(test)]) so the
// integration test binary can use them.
// ------------------------------------------------------------------------------------------------

/// Encode helpers that produce a real alloy-encoded `EncodedLog` (topics + ABI data) for each event,
/// so the MemLogSource path exercises the *same* `decode_log` the AlloyLogSource uses.
pub mod emit {
    use super::*;
    use alloy::primitives::{Address, U256};
    use alloy::sol_types::SolEvent;

    fn addr(a: &str) -> Address {
        a.parse().expect("addr")
    }
    fn b256(h: &str) -> B256 {
        let s = h.strip_prefix("0x").unwrap_or(h);
        let bytes = hex::decode(s).expect("b256 hex");
        B256::from_slice(&bytes)
    }

    fn encode<E: SolEvent>(contract: &str, ev: E) -> EncodedLog {
        let data = ev.encode_data();
        let mut topics: Vec<B256> = Vec::new();
        for t in ev.encode_topics() {
            topics.push(t.into());
        }
        EncodedLog {
            address: contract.to_ascii_lowercase(),
            topics,
            data,
        }
    }

    pub fn issuer_created(factory: &str, clone: &str, record_type: &str, name: &str) -> EncodedLog {
        encode(
            factory,
            IDogTagIssuerFactory::IssuerCreated {
                clone: addr(clone),
                recordType: b256(record_type),
                name: name.to_string(),
            },
        )
    }
    pub fn whitelisted(registry: &str, record_type: &str, signer: &str) -> EncodedLog {
        encode(
            registry,
            IIssuerRegistry::Whitelisted {
                recordType: b256(record_type),
                signer: addr(signer),
            },
        )
    }
    pub fn delisted(registry: &str, record_type: &str, signer: &str) -> EncodedLog {
        encode(
            registry,
            IIssuerRegistry::Delisted {
                recordType: b256(record_type),
                signer: addr(signer),
            },
        )
    }
    pub fn root_issued(clone: &str, root: &str, by: &str, ts: u64) -> EncodedLog {
        encode(
            clone,
            IDogTagIssuer::RootIssued {
                root: b256(root),
                by: addr(by),
                ts: U256::from(ts),
            },
        )
    }
    pub fn root_revoked(clone: &str, root: &str, by: &str, ts: u64) -> EncodedLog {
        encode(
            clone,
            IDogTagIssuer::RootRevoked {
                root: b256(root),
                by: addr(by),
                ts: U256::from(ts),
            },
        )
    }
    pub fn verified(
        vreg: &str,
        dog_tag_id: u64,
        relayer: &str,
        subject: &str,
        purpose: &str,
        nullifier: &str,
        ts: u64,
    ) -> EncodedLog {
        encode(
            vreg,
            IVerificationRegistry::Verified {
                dogTagId: U256::from(dog_tag_id),
                relayer: addr(relayer),
                subject: addr(subject),
                purpose: b256(purpose),
                nullifier: b256(nullifier),
                ts: U256::from(ts),
            },
        )
    }
}

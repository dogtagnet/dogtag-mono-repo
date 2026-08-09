//! On-device Groth16 proving (Workstream A) — UniFFI surface, gated behind the `prover` feature.
//!
//! This module lets the mobile app generate the owner-hidden consent Groth16 proof **locally**
//! (true ZK): the verifying operator never sees the witness, only relays
//! `{proof, publicSignals}` on-chain.
//!
//! It MIRRORS the ark-0.6 backend prover (`crates/dogtag-prover-rs/src/lib.rs`
//! `push_consent_inputs` + `format_output`) but does NOT depend on it — the backend stays on ark
//! 0.6, this crate stays on ark 0.5. Here we use `circom-prover` (ark 0.5 / mopro) with its
//! `circom-witnesscalc` GRAPH witness calculator (`WitnessFn::CircomWitnessCalc`) — a pure-Rust
//! interpreter of the circuit's field ops. We deliberately do NOT use `rust-witness` (wasm2c /
//! w2c2): it miscompiles the circuit's i64 BN254 field arithmetic on 32-bit ARM (armeabi-v7a),
//! zeroing the last-computed output wires. The graph calculator is integer-width-correct on any
//! target. The witness graph ships as a runtime asset (`consent.graph`), loaded by absolute file
//! path exactly like the zkey.
//!
//! The ASSEMBLY (`consent_assemble`) is shared with the prover-independent `assemble` feature
//! (compiled WITHOUT circom-prover) so the 64-bit backend can reuse the SAME assembly to drive the
//! server proving API. This module adds the circom-prover proving on top.

use std::sync::{Mutex, OnceLock};

use serde_json::Value;

use circom_prover::{
    prover::ProofLib,
    witness::WitnessFn,
    CircomProver,
};

use ark_bn254::Fr;
use ark_ff::PrimeField;

use crate::consent_assemble::{assemble_consent, consent_input_map, ConsentWitness};
use crate::ffi::{err, FfiError};
use crate::profile_tree::{AttributeLeaf, SALT_LEN};
use crate::types::TypeTag;
use crate::wrap::scalar_from_packed;

/// Number of public signals the consent circuit exposes
/// (`[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`).
const NUM_PUBLIC_CONSENT: usize = crate::public_signals::NUM_PUBLIC;

// ---------------------------------------------------------------------------------------------
// Graph witness calculator (`circom-witnesscalc`).
//
// `circom-prover` consumes a bare `fn(&str) -> anyhow::Result<Vec<u8>>` for the
// `WitnessFn::CircomWitnessCalc` variant — it cannot capture the graph path in a closure, and it
// runs the fn on a freshly spawned thread (so thread-locals on the caller don't reach it). We
// therefore stash the loaded graph bytes in a process-global cell keyed by absolute path, set by
// `prove_consent` right before it calls `CircomProver::prove`, and read by `graph_witness`.
//
// The graph (`consent.graph`, `wtns.graph.001` format) is a precompiled, target-independent
// description of the circuit's field ops; loading it once and reusing it is correct because the
// circuit is fixed. The bytes are interpreted in Rust by `circom_witnesscalc::calc_witness`, which
// has no i64 codegen and is therefore correct on 32-bit ARM where wasm2c was not.

/// The cached `(path, bytes)` of the most-recently-requested witness graph, guarded by a mutex.
type GraphCell = Mutex<Option<(String, &'static [u8])>>;

/// `(path, bytes)` of the most-recently-requested witness graph. Guarded by a mutex; the graph is
/// (re)loaded from disk only when the path changes (effectively once per process).
static GRAPH: OnceLock<GraphCell> = OnceLock::new();

fn graph_cell() -> &'static GraphCell {
    GRAPH.get_or_init(|| Mutex::new(None))
}

/// Load (and cache) the witness graph bytes for `path`, returning a `'static` slice. The bytes are
/// intentionally leaked once per distinct path so the `WitnessFn` fn — which runs on another thread
/// and takes no graph argument — can read them through the global cell.
fn load_graph(path: &str) -> Result<&'static [u8], FfiError> {
    let cell = graph_cell();
    let mut guard = cell.lock().map_err(|e| err(format!("graph cache poisoned: {e}")))?;
    if let Some((cached_path, bytes)) = guard.as_ref() {
        if cached_path == path {
            return Ok(bytes);
        }
    }
    let data = std::fs::read(path)
        .map_err(|e| err(format!("read witness graph {path}: {e}")))?;
    let leaked: &'static [u8] = Box::leak(data.into_boxed_slice());
    *guard = Some((path.to_string(), leaked));
    Ok(leaked)
}

/// The `WitnessFn::CircomWitnessCalc` entry point: interpret the cached witness graph over the
/// circuit `json_input` and return the serialized `.wtns` bytes circom-prover expects.
fn graph_witness(json_input: &str) -> anyhow::Result<Vec<u8>> {
    let bytes = {
        let cell = graph_cell();
        let guard = cell
            .lock()
            .map_err(|e| anyhow::anyhow!("graph cache poisoned: {e}"))?;
        match guard.as_ref() {
            Some((_, b)) => *b,
            None => anyhow::bail!("witness graph not loaded before prove"),
        }
    };
    circom_witnesscalc::calc_witness(json_input, bytes).map_err(|e| anyhow::anyhow!("{e}"))
}

/// Run a proving-dependency call with a PANIC BACKSTOP, downgrading any panic to a rendered
/// [`FfiError`].
///
/// This exists because `circom-prover 0.1.4` panics where it should return errors — receipts, from
/// the dependency's own source: `witness.rs:48` unwraps the witness callback's `Result` inside the
/// witness thread; `prover/arkworks.rs:38-41` maps the thread join to
/// `anyhow!("witness thread panicked")` and then UNWRAPS that; `:42` unwraps proving errors too. A
/// panic that escapes a `#[uniffi::export]` reaches the app as `rustPanic` — uncatchable, rendered
/// as a crash — which is exactly how a mismatched witness artifact surfaced on the captain's phone
/// (2026-08-09). A prover must never crash the app: it returns an error the app can render.
fn catch_prover_panic<T>(
    stage: &str,
    f: impl FnOnce() -> Result<T, FfiError>,
) -> Result<T, FfiError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(payload) => {
            let msg = payload
                .downcast_ref::<&str>()
                .map(|s| s.to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "non-string panic payload".to_string());
            Err(err(format!("{stage} panicked: {msg}")))
        }
    }
}

/// A Groth16 proof formatted exactly as the on-chain Solidity calldata expects (mirrors
/// `dogtag-prover-rs::Groth16Output`): `a`/`c` are G1 `[x,y]`; `b` is G2 with the snarkjs->Solidity
/// coordinate swap applied (`b[0]=[bx_c1,bx_c0]`, `b[1]=[by_c1,by_c0]`); `pub_signals` is the
/// 7-element output vector. All values are base-10 decimal strings.
#[derive(uniffi::Record)]
pub struct ProofFfi {
    pub a: Vec<String>,
    pub b: Vec<Vec<String>>,
    pub c: Vec<String>,
    pub pub_signals: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Owner-hidden CONSENT proving (M7 P0).
// ---------------------------------------------------------------------------------------------

/// Strip an optional `0x`, hex-decode, and require exactly `M` bytes.
fn decode_fixed<const M: usize>(label: &str, h: &str) -> Result<[u8; M], FfiError> {
    let s = h.strip_prefix("0x").unwrap_or(h);
    let bytes = hex::decode(s).map_err(|e| err(format!("bad {label} hex: {e}")))?;
    if bytes.len() != M {
        return Err(FfiError::Invalid(format!(
            "{label} must be {M} bytes (got {})",
            bytes.len()
        )));
    }
    let mut out = [0u8; M];
    out.copy_from_slice(&bytes);
    Ok(out)
}

/// Parse a 0x.. 32-byte big-endian word into a field element (reduces mod r, like the assembler seams).
fn field_from_word(label: &str, h: &str) -> Result<Fr, FfiError> {
    Ok(Fr::from_be_bytes_mod_order(&decode_fixed::<32>(label, h)?))
}

/// Parse the credential attribute leaves from JSON into [`AttributeLeaf`]s.
///
/// Shape: `[{ "keyPath": "credentialSubject.name", "salt": "0x..16B", "tag": 2, "value": "Rex" }, ...]`
/// (`tag` is the [`TypeTag`] byte; `value` is the raw scalar string the tag interprets). These are the
/// SAME disclosable attributes `build_profile_tree` folds into `R`, so they MUST match issuance.
fn parse_attributes(attributes_json: &str) -> Result<Vec<AttributeLeaf>, FfiError> {
    let arr: Value = serde_json::from_str(attributes_json)
        .map_err(|e| err(format!("bad attributes json: {e}")))?;
    let items = arr
        .as_array()
        .ok_or_else(|| FfiError::Invalid("attributes must be a JSON array".into()))?;
    let mut leaves = Vec::with_capacity(items.len());
    for (i, it) in items.iter().enumerate() {
        let get = |k: &str| -> Result<&str, FfiError> {
            it.get(k)
                .and_then(|v| v.as_str())
                .ok_or_else(|| FfiError::Invalid(format!("attribute[{i}].{k}: missing or not a string")))
        };
        let key_path = get("keyPath")?.to_string();
        let salt = decode_fixed::<SALT_LEN>(&format!("attribute[{i}].salt"), get("salt")?)?;
        let tag_n = it
            .get("tag")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| FfiError::Invalid(format!("attribute[{i}].tag: missing or not a number")))?;
        let tag = TypeTag::from_u8(tag_n as u8)
            .ok_or_else(|| FfiError::Invalid(format!("attribute[{i}].tag {tag_n} is not a valid TypeTag")))?;
        let value = scalar_from_packed(tag, get("value")?).map_err(FfiError::from)?;
        leaves.push(AttributeLeaf {
            key_path,
            salt,
            value,
        });
    }
    Ok(leaves)
}

/// The owned parts a consent proof needs, parsed from the FFI's string params. Held separately from
/// [`ConsentWitness`] (which borrows) so the parsing can be unit-tested without proving.
struct ConsentFfiInputs {
    seed: Vec<u8>,
    dog_tag_id_handle: String,
    owner_address: [u8; 20],
    attributes: Vec<AttributeLeaf>,
    purpose: Fr,
    relayer: [u8; 20],
    record_type: Fr,
    deadline: Fr,
    consent_nonce: Fr,
}

impl ConsentFfiInputs {
    /// Borrow the owned parts into a [`ConsentWitness`] the assembler consumes.
    fn witness(&self) -> ConsentWitness<'_> {
        ConsentWitness {
            seed: &self.seed,
            dog_tag_id_handle: &self.dog_tag_id_handle,
            owner_address: self.owner_address,
            attributes: &self.attributes,
            purpose: self.purpose,
            relayer: self.relayer,
            record_type: self.record_type,
            deadline: self.deadline,
            consent_nonce: self.consent_nonce,
        }
    }
}

/// Parse the FFI's string params into the owned [`ConsentFfiInputs`] (fail-closed on any bad hex /
/// length / attribute shape). Kept separate from the prove so it is hermetically testable.
#[allow(clippy::too_many_arguments)]
fn parse_consent_ffi_inputs(
    seed_hex: &str,
    dog_tag_id_handle: &str,
    owner_address_hex: &str,
    attributes_json: &str,
    purpose_hex: &str,
    relayer_hex: &str,
    record_type_hex: &str,
    consent_nonce_hex: &str,
    deadline_dec: &str,
) -> Result<ConsentFfiInputs, FfiError> {
    let seed = {
        let s = seed_hex.strip_prefix("0x").unwrap_or(seed_hex);
        hex::decode(s).map_err(|e| err(format!("bad seed hex: {e}")))?
    };
    Ok(ConsentFfiInputs {
        seed,
        dog_tag_id_handle: dog_tag_id_handle.to_string(),
        owner_address: decode_fixed::<20>("ownerAddress", owner_address_hex)?,
        attributes: parse_attributes(attributes_json)?,
        purpose: field_from_word("purpose", purpose_hex)?,
        relayer: decode_fixed::<20>("relayer", relayer_hex)?,
        record_type: field_from_word("recordType", record_type_hex)?,
        consent_nonce: field_from_word("consentNonce", consent_nonce_hex)?,
        deadline: Fr::from(
            deadline_dec
                .parse::<u128>()
                .map_err(|e| err(format!("bad deadline decimal: {e}")))?,
        ),
    })
}

/// Generate a Groth16 proof for the DogTag CONSENT circuit (`consent.circom`, `DogTagConsent(6)`) ON
/// DEVICE (M7 P0).
///
/// Uses the `circom-witnesscalc` GRAPH backend (deliberately not rust-witness/wasm2c, which
/// miscompiles i64 field math on 32-bit ARM) for the owner-unlinkable consent circuit: it ASSEMBLES
/// the inputs with [`assemble_consent`] (the canonical `dogTagId` field is computed once and used
/// for both the circuit input and the `build_profile_tree` KDF binding), then proves.
///
/// - `seed_hex`            — the owner wallet seed (0x..); owner-secret/consent-key/salts derive from it.
/// - `dog_tag_id_handle`   — the off-chain decimal handle; field-hashed to the canonical `dogTagId`.
/// - `owner_address_hex`   — 0x.. 20-byte owner address (the owner-address reserved leaf value).
/// - `attributes_json`     — the disclosable credential attributes (see [`parse_attributes`]); MUST
///   match issuance so the rebuilt `R` equals the minted `profileRoot`.
/// - `purpose_hex` / `record_type_hex` / `consent_nonce_hex` — 0x.. 32-byte field words.
/// - `relayer_hex`         — 0x.. 20-byte relayer address (range-checked `< 2^160` by the circuit).
/// - `deadline_dec`        — the consent expiry as a decimal string.
/// - `zkey_path` / `graph_path` — `consent_final.zkey` + `consent.graph` (bundled/fetched app assets).
///
/// Returns the proof as Solidity calldata plus the 7 public signals in the FROZEN OUTPUT order
/// `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]` (all decimal).
#[allow(clippy::too_many_arguments)]
#[uniffi::export]
pub fn prove_consent(
    seed_hex: String,
    dog_tag_id_handle: String,
    owner_address_hex: String,
    attributes_json: String,
    purpose_hex: String,
    relayer_hex: String,
    record_type_hex: String,
    consent_nonce_hex: String,
    deadline_dec: String,
    zkey_path: String,
    graph_path: String,
) -> Result<ProofFfi, FfiError> {
    let parsed = parse_consent_ffi_inputs(
        &seed_hex,
        &dog_tag_id_handle,
        &owner_address_hex,
        &attributes_json,
        &purpose_hex,
        &relayer_hex,
        &record_type_hex,
        &consent_nonce_hex,
        &deadline_dec,
    )?;

    let inp = assemble_consent(&parsed.witness())?;
    let input_json = serde_json::to_string(&consent_input_map(&inp))
        .map_err(|e| err(format!("serialize consent circuit input: {e}")))?;

    // Graph-witness plumbing: cached per path, read on circom-prover's own thread through the
    // process-global cell.
    let graph = load_graph(&graph_path)?;

    // PRE-FLIGHT the witness on THIS thread, behind the panic backstop, before any proving
    // machinery runs. Inside `CircomProver::prove` the same calculation happens on a spawned
    // thread whose failures are UNWRAPPED (circom-prover 0.1.4 `witness.rs:48`,
    // `prover/arkworks.rs:41` — and `circom_witnesscalc` itself unwraps graph parsing at
    // `lib.rs:165`), so an input the calculator rejects — or a graph from another generation, the
    // reproduced "Invalid magic" case — reached the app as an uncatchable `rustPanic` instead of
    // an error it can render (measured on the captain's phone, 2026-08-09). A successful run here
    // is deterministic proof the dependency's own witness thread will succeed on the identical
    // bytes; the calculation is milliseconds against a proving step of tens of seconds.
    if let Err(cause) = catch_prover_panic("consent witness calculation", || {
        circom_witnesscalc::calc_witness(&input_json, graph)
            .map(drop)
            .map_err(|e| err(format!("consent witness rejected: {e}")))
    }) {
        return Err(err(format!(
            "{cause} — the assembled circuit input and the bundled witness graph ({graph_path}) \
             do not agree ({} attribute leaves + 3 reserved inclusion paths were assembled). If \
             this credential or profile was recorded under a superseded deployment or artifact \
             set, re-import or re-issue it on the current one; if the artifact bundle is stale, \
             rebuild and reinstall the app.",
            parsed.attributes.len()
        )));
    }

    let proof = catch_prover_panic("circom-prover prove consent", || {
        CircomProver::prove(
            ProofLib::Arkworks,
            WitnessFn::CircomWitnessCalc(graph_witness),
            input_json.clone(),
            zkey_path.clone(),
        )
        .map_err(|e| err(format!("circom-prover prove consent: {e}")))
    })?;

    let pub_signals: Vec<String> = proof.pub_inputs.0.iter().map(|b| b.to_string()).collect();
    if pub_signals.len() != NUM_PUBLIC_CONSENT {
        return Err(FfiError::Invalid(format!(
            "unexpected consent public-signal count: got {}, expected {NUM_PUBLIC_CONSENT}",
            pub_signals.len()
        )));
    }

    let (a_t, b_t, c_t) = proof.proof.as_tuple();
    let a = vec![a_t.0.to_string(), a_t.1.to_string()];
    let c = vec![c_t.0.to_string(), c_t.1.to_string()];
    let b = vec![
        vec![b_t.0[0].to_string(), b_t.0[1].to_string()],
        vec![b_t.1[0].to_string(), b_t.1[1].to_string()],
    ];

    Ok(ProofFfi {
        a,
        b,
        c,
        pub_signals,
    })
}

#[cfg(test)]
mod consent_tests {
    use super::*;
    use crate::types::TypedScalar;

    fn word32(hi: u64) -> String {
        let mut w = [0u8; 32];
        w[24..].copy_from_slice(&hi.to_be_bytes());
        format!("0x{}", hex::encode(w))
    }

    /// THE CAPTAIN'S CRASH (live walk 2026-08-09), reproduced and pinned: proving against a
    /// witness graph that does not match this build's circuit — any artifact from a superseded
    /// generation, or a corrupt copy — used to reach the app as an UNCATCHABLE `rustPanic`
    /// ("called `Result::unwrap()` on an `Err` value" / "witness thread panicked"), because
    /// `circom-prover 0.1.4` unwraps the witness callback's `Result` inside its witness thread
    /// (`witness.rs:48`) and unwraps the thread join again (`prover/arkworks.rs:38-41`). Verified
    /// red before the pre-flight existed: this exact test died with that panic instead of
    /// returning `Err`.
    ///
    /// The prover's contract is: NEVER a panic — an inconsistent input comes back as a rendered
    /// error naming what was rejected.
    #[test]
    fn a_mismatched_witness_graph_is_a_named_error_never_a_panic() {
        let seed = b"ffi graph mismatch test wallet seed - TEST MATERIAL ONLY".to_vec();
        let salt = [7u8; SALT_LEN];
        let attributes_json = format!(
            r#"[{{"keyPath":"credentialSubject.name","salt":"0x{}","tag":2,"value":"Rex"}}]"#,
            hex::encode(salt)
        );
        // A graph file from "another generation": bytes that are not THIS build's consent.graph.
        let dir = std::env::temp_dir().join(format!("dogtag-graph-mismatch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let bad_graph = dir.join("not-the-consent.graph");
        std::fs::write(&bad_graph, b"wtns.graph.999 this is not a witness graph").expect("write");

        // The committed zkey; post-fix it is never read (the witness pre-flight refuses first),
        // and its absence would mean an incomplete checkout.
        let zkey = "../../circuits/build/consent_final.zkey";
        assert!(
            std::path::Path::new(zkey).exists(),
            "incomplete checkout: {zkey} is committed and must exist"
        );

        let res = prove_consent(
            format!("0x{}", hex::encode(&seed)),
            "424242".into(),
            format!("0x{}", hex::encode([0xabu8; 20])),
            attributes_json,
            word32(1),
            format!("0x{}", hex::encode([0x11u8; 20])),
            word32(2),
            word32(3),
            "1893456000".into(),
            zkey.into(),
            bad_graph.to_string_lossy().into_owned(),
        );
        match res {
            Err(FfiError::Invalid(m)) => {
                assert!(
                    m.contains("witness"),
                    "the refusal names the rejected stage: {m}"
                );
                assert!(
                    m.contains("superseded deployment") || m.contains("artifact"),
                    "the refusal points at the mismatched-artifact cause: {m}"
                );
            }
            Ok(_) => panic!("a mismatched graph must not prove"),
        }
    }

    /// The panic backstop itself: a panic inside the proving dependency is downgraded to a
    /// rendered error, never re-raised across the FFI.
    #[test]
    fn a_panicking_prover_call_is_downgraded_to_a_rendered_error() {
        let r: Result<(), FfiError> =
            catch_prover_panic("test stage", || panic!("boom from the dependency"));
        match r {
            Err(FfiError::Invalid(m)) => {
                assert!(m.contains("test stage"), "{m}");
                assert!(m.contains("boom from the dependency"), "{m}");
            }
            _ => panic!("expected the panic to arrive as FfiError::Invalid"),
        }
    }

    /// The FFI param parsing produces exactly the witness a caller would build in-Rust: parse the
    /// string params, assemble, and assert the resulting `R` / canonical `dogTagId` match a directly
    /// constructed [`ConsentWitness`] with the same values. Hermetic — no proving, no graph.
    #[test]
    fn parse_consent_ffi_inputs_round_trips_to_the_same_witness() {
        let seed = b"ffi parse test wallet seed - TEST MATERIAL ONLY".to_vec();
        let owner_address = [0xabu8; 20];
        let salt = [7u8; SALT_LEN];
        let attributes_json = format!(
            r#"[{{"keyPath":"credentialSubject.name","salt":"0x{}","tag":2,"value":"Rex"}}]"#,
            hex::encode(salt)
        );

        let parsed = parse_consent_ffi_inputs(
            &format!("0x{}", hex::encode(&seed)),
            "424242",
            &format!("0x{}", hex::encode(owner_address)),
            &attributes_json,
            &word32(7),
            "0x1111111111111111111111111111111111111111",
            &word32(19),
            &word32(99),
            "1893456000",
        )
        .expect("parse consent ffi inputs");

        let from_ffi = assemble_consent(&parsed.witness()).expect("assemble from parsed");

        // A directly-built witness with the SAME values must assemble identically.
        let direct_attrs = vec![AttributeLeaf {
            key_path: "credentialSubject.name".to_string(),
            salt,
            value: TypedScalar::Str("Rex".to_string()),
        }];
        let direct = assemble_consent(&ConsentWitness {
            seed: &seed,
            dog_tag_id_handle: "424242",
            owner_address,
            attributes: &direct_attrs,
            purpose: Fr::from(7u64),
            relayer: [0x11u8; 20],
            record_type: Fr::from(19u64),
            deadline: Fr::from(1_893_456_000u64),
            consent_nonce: Fr::from(99u64),
        })
        .expect("assemble direct");

        assert_eq!(from_ffi.root, direct.root, "parsed witness must bind the same R");
        assert_eq!(from_ffi.dog_tag_id_field, direct.dog_tag_id_field);
        assert_eq!(from_ffi.dog_tag_id, direct.dog_tag_id);
        assert_eq!(from_ffi.nullifier, direct.nullifier);
    }

    /// Fail-closed parsing: bad length, an invalid attribute tag, and a bad deadline are all rejected.
    #[test]
    fn parse_consent_ffi_inputs_is_fail_closed() {
        let ok_addr = format!("0x{}", "ab".repeat(20));
        let parse = |over: &str| -> Result<ConsentFfiInputs, FfiError> {
            parse_consent_ffi_inputs(
                "0xdeadbeef",
                "1",
                if over == "addr" { "0x00" } else { &ok_addr },
                if over == "attr_tag" {
                    r#"[{"keyPath":"k","salt":"0x00000000000000000000000000000000","tag":9,"value":"v"}]"#
                } else {
                    "[]"
                },
                &word32(1),
                if over == "relayer" { "0xff" } else { "0x1111111111111111111111111111111111111111" },
                &word32(1),
                &word32(1),
                if over == "deadline" { "not-a-number" } else { "1" },
            )
        };
        assert!(parse("ok").is_ok(), "the baseline inputs must parse");
        for bad in ["addr", "attr_tag", "relayer", "deadline"] {
            assert!(parse(bad).is_err(), "the `{bad}` mutation must be rejected");
        }
    }
}

//! On-device Groth16 proving (Workstream A) — UniFFI surface, gated behind the `prover` feature.
//!
//! This module lets the mobile app generate the verification Groth16 proof **locally** (true ZK):
//! the groomer never sees the raw record, only relays `{proof, publicSignals, consent}` on-chain.
//!
//! It MIRRORS the ark-0.6 backend prover (`crates/dogtag-prover-rs/src/lib.rs`
//! `push_named_inputs` + `format_output`) but does NOT depend on it — the backend stays on ark 0.6,
//! this crate stays on ark 0.5. Here we use `circom-prover` (ark 0.5 / mopro) with its
//! `circom-witnesscalc` GRAPH witness calculator (`WitnessFn::CircomWitnessCalc`) — a pure-Rust
//! interpreter of the circuit's field ops. We deliberately do NOT use `rust-witness` (wasm2c /
//! w2c2): it miscompiles the circuit's i64 BN254 field arithmetic on 32-bit ARM (armeabi-v7a),
//! zeroing the last-computed output wires (nullifier/keyHash). The graph calculator is integer-
//! width-correct on any target. The witness graph ships as a runtime asset
//! (`verification.graph`), loaded by absolute file path exactly like the zkey.
//!
//! The circuit `DogTagVerification(24, 5)` takes 19 named inputs and emits 7 public outputs in the
//! snarkjs order `[dogTagId, purpose, relayer, subject, nullifier, keyHash, R]`. We ASSEMBLE all 19
//! inputs from this crate's own internals (wrap/leaf/field/merkle) + the passed-through EdDSA sig,
//! prove, then format the proof with the snarkjs->Solidity b-coordinate swap so the calldata drops
//! straight into `recordVerificationZK`.
//!
//! The ASSEMBLY (`assemble` / `input_map` / `EddsaSigInput`) is shared with the prover-independent
//! `prover_assemble` module (compiled under the lighter `assemble` feature, WITHOUT circom-prover) so
//! the 64-bit backend can reuse the SAME 19-input assembly to drive the server proving API — see
//! `prover_assemble::assemble_circuit_input`. This module adds the circom-prover proving on top.

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
use crate::ffi::FfiError;
// Reuse the prover-independent assembly (shared with the server proving path).
use crate::prover_assemble::{assemble, consent_from_json, err, input_map};
use crate::profile_tree::{AttributeLeaf, SALT_LEN};
use crate::types::TypeTag;
use crate::wrap::{scalar_from_packed, WrappedDoc};

// Re-export `EddsaSigInput` at the historical `prover_ffi::EddsaSigInput` path so existing consumers
// (e.g. the `prove_parity` live regression test, the generated UniFFI bindings) keep working — it
// now physically lives in `prover_assemble` (shared with the server proving path).
pub use crate::prover_assemble::EddsaSigInput;

/// Number of public signals the Level-A verification circuit exposes.
const NUM_PUBLIC: usize = 7;

/// Number of public signals the Level-B consent circuit exposes
/// (`[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`).
const NUM_PUBLIC_CONSENT: usize = 7;

// ---------------------------------------------------------------------------------------------
// Graph witness calculator (`circom-witnesscalc`).
//
// `circom-prover` consumes a bare `fn(&str) -> anyhow::Result<Vec<u8>>` for the
// `WitnessFn::CircomWitnessCalc` variant — it cannot capture the graph path in a closure, and it
// runs the fn on a freshly spawned thread (so thread-locals on the caller don't reach it). We
// therefore stash the loaded graph bytes in a process-global cell keyed by absolute path, set by
// `prove_verification` right before it calls `CircomProver::prove`, and read by `graph_witness`.
//
// The graph (`verification.graph`, `wtns.graph.001` format) is a precompiled, target-independent
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

/// Generate a Groth16 proof for the DogTag verification circuit ON DEVICE.
///
/// - `wrapped_doc_json` — the stored WrappedDoc (raw salted leaves; the witness source).
/// - `consent_json`     — the signed consent (same hex shape as the POSTed consent / ffi.rs consent).
/// - `eddsa_sig`        — the EdDSA-BabyJubjub consent signature + public key.
/// - `zkey_path`        — filesystem path to `verification_final.zkey` (bundled app asset).
/// - `graph_path`       — filesystem path to `verification.graph`, the precompiled witness graph
///   (bundled app asset, loaded the same way as the zkey).
///
/// Returns the proof as Solidity calldata (`a`, `b` with the snarkjs->Solidity swap, `c`) plus the
/// 7 public signals `[dogTagId, purpose, relayer, subject, nullifier, keyHash, R]` (all decimal).
#[uniffi::export]
pub fn prove_verification(
    wrapped_doc_json: String,
    consent_json: String,
    eddsa_sig: EddsaSigInput,
    zkey_path: String,
    graph_path: String,
) -> Result<ProofFfi, FfiError> {
    let doc: WrappedDoc = serde_json::from_str(&wrapped_doc_json)
        .map_err(|e| err(format!("bad wrapped doc json: {e}")))?;
    let consent_v: Value =
        serde_json::from_str(&consent_json).map_err(|e| err(format!("bad consent json: {e}")))?;
    let consent = consent_from_json(&consent_v)?;

    // Assemble the 19 named circuit inputs (shared with the server proving path).
    let inp = assemble(&doc, &consent, &eddsa_sig)?;
    let input_json = serde_json::to_string(&input_map(&inp))
        .map_err(|e| err(format!("serialize circuit input: {e}")))?;

    // Load the witness graph (cached per path) so the `graph_witness` fn — invoked by circom-prover
    // on its own thread, with no graph argument — can read it through the process-global cell.
    load_graph(&graph_path)?;

    let proof = CircomProver::prove(
        ProofLib::Arkworks,
        WitnessFn::CircomWitnessCalc(graph_witness),
        input_json,
        zkey_path,
    )
    .map_err(|e| err(format!("circom-prover prove: {e}")))?;

    // Public signals come back in circuit-output order (snarkjs order). Assert count.
    let pub_signals: Vec<String> = proof.pub_inputs.0.iter().map(|b| b.to_string()).collect();
    if pub_signals.len() != NUM_PUBLIC {
        return Err(FfiError::Invalid(format!(
            "unexpected public-signal count: got {}, expected {NUM_PUBLIC}",
            pub_signals.len()
        )));
    }

    // Format the proof with the snarkjs->Solidity b-coordinate swap (mirrors
    // dogtag-prover-rs::format_output). circom-prover stores G2 as x=[c0,c1]; `as_tuple()` already
    // emits the swapped [c1, c0] form, so we read it directly.
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

// ---------------------------------------------------------------------------------------------
// Level-B CONSENT proving (M7 P0).
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
/// Mirrors [`prove_verification`] (same `circom-witnesscalc` GRAPH backend — deliberately not
/// rust-witness/wasm2c, which miscompiles i64 field math on 32-bit ARM), but for the Level-B
/// owner-unlinkable consent circuit: it ASSEMBLES the inputs with [`assemble_consent`] (the canonical
/// `dogTagId` field is computed once and used for both the circuit input and the `build_profile_tree`
/// KDF binding), then proves.
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

    // Same graph-witness plumbing as `prove_verification` (cached per path, read on circom-prover's
    // own thread through the process-global cell).
    load_graph(&graph_path)?;

    let proof = CircomProver::prove(
        ProofLib::Arkworks,
        WitnessFn::CircomWitnessCalc(graph_witness),
        input_json,
        zkey_path,
    )
    .map_err(|e| err(format!("circom-prover prove consent: {e}")))?;

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

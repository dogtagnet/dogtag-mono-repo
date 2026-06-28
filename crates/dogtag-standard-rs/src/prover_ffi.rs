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

use crate::ffi::FfiError;
// Reuse the prover-independent assembly (shared with the server proving path).
use crate::prover_assemble::{assemble, consent_from_json, err, input_map};
use crate::wrap::WrappedDoc;

// Re-export `EddsaSigInput` at the historical `prover_ffi::EddsaSigInput` path so existing consumers
// (e.g. the `prove_parity` live regression test, the generated UniFFI bindings) keep working — it
// now physically lives in `prover_assemble` (shared with the server proving path).
pub use crate::prover_assemble::EddsaSigInput;

/// Number of public signals the circuit exposes.
const NUM_PUBLIC: usize = 7;

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

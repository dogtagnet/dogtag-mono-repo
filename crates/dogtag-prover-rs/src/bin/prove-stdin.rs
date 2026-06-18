//! e2e helper: read a circuit-input JSON object from stdin (the `input` object emitted
//! by scripts/zk/gen_input.mjs), load the prover from CIRCUITS_BUILD_DIR (default
//! ../../circuits/build), generate a REAL Groth16 proof, and print the
//! `{a,b,c,pub}` Solidity-calldata output as JSON on stdout.
//!
//! Used by scripts/e2e-zk.sh to produce the on-device-style proof the relayer broadcasts.

use std::io::Read;
use std::path::PathBuf;

use dogtag_prover::{ProveInputs, Prover};

fn main() {
    let build_dir = std::env::var("CIRCUITS_BUILD_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            // crate dir = <root>/crates/dogtag-prover-rs
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .parent()
                .unwrap()
                .join("circuits")
                .join("build")
        });

    let mut buf = String::new();
    std::io::stdin()
        .read_to_string(&mut buf)
        .expect("read stdin");
    let v: serde_json::Value = serde_json::from_str(&buf).expect("parse input json");
    // Accept either the bare input object or {input: {...}}.
    let input_v = v.get("input").cloned().unwrap_or(v);
    let inputs = ProveInputs::from_circuit_input_json(&input_v).expect("ProveInputs");

    let prover = Prover::load(&build_dir).expect("load prover artifacts");
    let out = prover.prove(inputs).expect("prove");

    println!("{}", serde_json::to_string(&out).expect("serialize output"));
}

//! Build script for `dogtag-standard-rs`.
//!
//! It does NOTHING unless the OFF-by-default `prover` feature is active. This is load-bearing:
//! the default workspace build (vet-api / admin-api depend on this crate) must stay fast and must
//! NOT transpile the ~4.3 MB vendored circuit wasm into native C. We therefore gate every action
//! on `CARGO_FEATURE_PROVER`.
//!
//! When `prover` IS active, we transpile the vendored `circuit/verification.wasm` into native C via
//! `rust_witness::transpile::transpile_wasm` (mopro/zkmopro, wasm2c via w2c2 — no wasmer, App-Store
//! safe). The witness fn the C exposes is consumed by `rust_witness::witness!(verification)` in
//! `src/prover_ffi.rs` and fed to `circom-prover`.

fn main() {
    // Hard gate: when the prover feature is OFF, emit nothing and return immediately.
    if std::env::var("CARGO_FEATURE_PROVER").is_err() {
        return;
    }

    #[cfg(feature = "prover")]
    {
        // The directory holding the vendored `verification.wasm`. `transpile_wasm` iterates every
        // `*.wasm` under this dir; the file stem (`verification`) becomes the C symbol prefix that
        // `witness!(verification)` links against.
        let circuit_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("circuit");
        println!("cargo:rerun-if-changed={}", circuit_dir.display());
        rust_witness::transpile::transpile_wasm(circuit_dir.to_string_lossy().into_owned());
    }
}

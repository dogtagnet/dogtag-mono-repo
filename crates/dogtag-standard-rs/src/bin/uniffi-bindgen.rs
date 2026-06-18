//! UniFFI binding generator entrypoint (matches the crate's uniffi 0.28 dep exactly, avoiding
//! version skew). Build with `--features uniffi/cli` and run e.g.:
//!   cargo run --features uniffi/cli --bin uniffi-bindgen -- generate \
//!     --library target/.../libdogtag_standard.dylib --language kotlin --out-dir <dir>
fn main() {
    uniffi::uniffi_bindgen_main()
}

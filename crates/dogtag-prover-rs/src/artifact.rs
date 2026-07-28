//! Version-keyed proving artifacts (M7 §3.2).
//!
//! A protocol **version key** (e.g. [`LEVEL_B_V1`]) resolves to an [`ArtifactDescriptor`]: the set of
//! files a prover needs for that version, plus the integrity pins to check them against. This replaces
//! a single hard-coded artifact set — a pinned zkey filename + one expected hash — with a table that
//! can hold many versions.
//!
//! # What this module is, and is NOT
//!
//! It is the **structure** fully-dynamic proving (M7 lock C) needs: per-version artifact identity +
//! per-version integrity pins. It is **not** the fetch. Every descriptor here resolves to artifacts
//! that already exist locally (the server's `circuits/build`, the app's bundle); nothing is
//! downloaded. Wiring the descriptor's files to a network fetch — pinning the bytes against
//! [`ArtifactFile::sha256`] before load, exactly as the zkey already is — is the later discovery
//! workstream. The descriptor is the seam that work plugs into.
//!
//! # `zkey.sha256` is NOT the VK hash
//!
//! Two different things, deliberately carried as two fields (M7 §3.2; ZK cross-check §2):
//!
//! * [`ZkeyArtifact::sha256`] — the **fetch/integrity pin**: SHA-256 of the proving-key *file*. It is
//!   what the loader hashes the bytes against BEFORE parsing them, so a swapped or
//!   corrupt key fails closed (audit M4).
//! * [`VerifyingKeyIdentity`] — which **VK** the resulting proof verifies against. The authoritative
//!   VK is the one embedded in the on-chain `Groth16VerifierConsent`, identified by its address; the
//!   `consent_verification_key.json` hash pinned here identifies the same VK off-chain. It is NOT a
//!   fetch pin and the prover never loads that file (it uses the VK inside the zkey).
//!
//! Conflating them would let a version publish one hash and imply the other, so they never share a
//! field.
//!
//! # Adding a version
//!
//! Add an [`ArtifactDescriptor`] const and register it in [`REGISTRY`]. The sole entry today is the
//! owner-hidden consent version ([`LEVEL_B_V1_DESCRIPTOR`]): its code path is
//! [`crate::ConsentProveInputs`] / [`crate::Prover::prove_consent_inputs`], fed by the SDK's
//! `consent_assemble` assembler. It is built from `circuits/consent.circom` and pins its own zkey
//! AND its own VK, separately.

/// The owner-hidden consent circuit, version 1 (M7 P0) - the sole and default protocol version.
///
/// The literal string is an INTERNAL version key (its keccak is the on-chain `contractSetId`), not a
/// user-facing label - it is never renamed, even though the "level" vocabulary is retired everywhere
/// user-facing. Proofs verify against the frozen `Groth16VerifierConsent` VK; inputs are the
/// [`crate::ConsentProveInputs`] shape, and the public-signal vector is
/// `[dogTagId, purpose, relayer, nullifier, R, recordType, deadline]`.
pub const LEVEL_B_V1: &str = "dogtag-levelb/1";

/// A file an artifact set is made of, addressed relative to a build/bundle directory.
///
/// `sha256` is the integrity pin (lowercase hex), checked before the file is parsed when the loader
/// reads it. `None` means **this version pins no hash for this file** — not "any bytes will do", but
/// "no pin has been published". The loader treats a `None` pin as unpinned and skips the check, so
/// only pin what is genuinely byte-stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactFile {
    /// Path relative to the artifact root (the server's `circuits/build`, the app's bundle).
    pub rel_path: &'static str,
    /// SHA-256 (lowercase hex) of the file, or `None` if this version publishes no pin for it.
    pub sha256: Option<&'static str>,
}

/// The Groth16 proving key and its **mandatory** integrity pin.
///
/// Separate from [`ArtifactFile`] because the zkey pin is not optional: it is the ceremony-bound
/// artifact whose substitution would silently produce proofs against the wrong key (audit M4), so
/// the type makes "a version with an unpinned zkey" unrepresentable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ZkeyArtifact {
    /// Path relative to the artifact root.
    pub rel_path: &'static str,
    /// SHA-256 (lowercase hex) of the zkey file. Always checked before the parse.
    pub sha256: &'static str,
}

/// Which verifying key a version's proofs check against — an **identity**, not a fetch pin.
///
/// See the module docs: the authoritative VK lives in the on-chain verifier at `verifier`'s address;
/// `verification_key_json.sha256` identifies the same VK as an off-chain file. Kept distinct from
/// [`ZkeyArtifact::sha256`] so the two can never be conflated (ZK cross-check §2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VerifyingKeyIdentity {
    /// The snarkjs-exported `verification_key.json` for this version, relative to the artifact root.
    ///
    /// The prover does not read this file (the VK it proves with is embedded in the zkey); it is the
    /// off-chain identity of the VK, and what the crate's tests check the on-chain verifier against.
    pub verification_key_json: ArtifactFile,
}

/// Everything a prover needs for ONE protocol version.
///
/// The sole entry is [`LEVEL_B_V1_DESCRIPTOR`]. Resolve one with [`resolve`] / [`current`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactDescriptor {
    /// The version key ([`LEVEL_B_V1`]) — the string callers pass over the wire.
    pub version: &'static str,
    /// The circuit this version proves, as `<source>/<template>(<params>)`.
    pub circuit_id: &'static str,
    /// How many public signals the circuit exposes.
    pub num_public: usize,
    /// The public-signal vector in snarkjs order — the meaning of each slot of
    /// [`crate::Groth16Output::public_signals`].
    pub public_signal_layout: &'static [&'static str],
    /// The proving key (mandatory pin).
    pub zkey: ZkeyArtifact,
    /// The compiled constraint system — the server's ark witness backend needs it.
    pub r1cs: ArtifactFile,
    /// The wasm witness calculator — the server's ark witness backend needs it.
    pub wasm: ArtifactFile,
    /// The precompiled witness graph — the MOBILE witness backend needs it (`circom-witnesscalc`'s
    /// pure-Rust graph interpreter, deliberately not wasm: wasm2c miscompiles i64 BN254 field
    /// arithmetic on 32-bit ARM). The server never loads it; the mobile resolvers do.
    pub witness_graph: ArtifactFile,
    /// Which VK this version's proofs verify against (identity — NOT the zkey pin).
    pub vk: VerifyingKeyIdentity,
}

/// SHA-256 of the committed `circuits/build/consent.graph` — the witness graph the MOBILE prover
/// interprets, and the value the on-chain `ArtifactSet.witnessMobileSha256` now carries.
///
/// # Why this is a named constant rather than a literal in the descriptor
///
/// The graph used to be an out-of-band local build that no checkout was guaranteed to have, so
/// nothing could attest which graph an app proved with (audit M9 rec 10). It is now **committed**,
/// which makes its bytes fixed and this hash checkable — `graph_file_matches_attested_sha256`
/// enforces that on every test run. Naming it once keeps the descriptor pin, the signed manifest and
/// `scripts/vendor-mobile-artifacts.sh` reading the same value instead of three transcriptions.
///
/// # The lockstep, now closed on both sides
///
/// [`ArtifactDescriptor::witness_graph`] feeds [`crate::manifest::Manifest::witness_mobile_sha256`],
/// and [`crate::manifest::reconcile`] treats a manifest `Some` against an on-chain `0`/`None` as a
/// CONFLICT. Both sides were flipped together on 2026-07-28: `dogtag-levelb-artifacts/1` was
/// re-published in place on ROAX carrying this hash, and the descriptor pin became
/// `Some(LEVEL_B_V1_WITNESS_GRAPH_SHA256)` in the same change. Moving ONE side in isolation - in
/// either direction - makes every reconcile report a disagreement that is really a half-applied
/// rollout, so a future rotation moves the constant, the descriptor and the chain together
/// (`docs/ARTIFACT_PIN_RUNBOOK.md`).
///
/// This publishes the graph's identity; it does NOT make it app-enforced. The mobile resolvers do not
/// hash the bundled graph and do not decode `witnessMobileSha256` at all (`AnchorResolver` reads only
/// `artifactSetId`, `minAppVersion` and `active`), so an app shipping a divergent graph still would
/// not detect it at runtime. Bundled-artifact integrity remains the package signature's job.
pub const LEVEL_B_V1_WITNESS_GRAPH_SHA256: &str =
    "2f74d26b800230400639e92211d80ff453bf82c2057b788fa1350e009748f793";

/// The owner-hidden consent artifact set (M7 P0) — the sole entry, and the default for every caller
/// naming no version.
///
/// The zkey/VK are the frozen M3 testnet-grade ceremony output (`docs/CEREMONY_TRANSCRIPT.consent.md`,
/// committed under `circuits/build`). `DogTagConsent(6)` feeds depth-6 inclusion PATHS via
/// [`crate::ConsentProveInputs`]. Its public-signal layout is the frozen seven-OUTPUT order; the
/// witness graph is committed too, and its bytes are attested by
/// [`LEVEL_B_V1_WITNESS_GRAPH_SHA256`].
pub const LEVEL_B_V1_DESCRIPTOR: ArtifactDescriptor = ArtifactDescriptor {
    version: LEVEL_B_V1,
    circuit_id: "consent.circom/DogTagConsent(6)",
    num_public: crate::NUM_PUBLIC,
    public_signal_layout: &[
        "dogTagId",
        "purpose",
        "relayer",
        "nullifier",
        "R",
        "recordType",
        "deadline",
    ],
    zkey: ZkeyArtifact {
        rel_path: "consent_final.zkey",
        sha256: "f83a111fcf233f42bc1c9e7282796a7eca3a9a52760ad7e35c0036b8eb36c868",
    },
    r1cs: ArtifactFile {
        rel_path: "consent.r1cs",
        sha256: Some("828e2923a159b04f2de421d4b447f8c85356677f4f83a5af55b42eb2b4f9b6b7"),
    },
    wasm: ArtifactFile {
        rel_path: "consent_js/consent.wasm",
        sha256: Some("482debcff5a4325c008dd00e4476bba011d0a706da955e3129d114f996a913e6"),
    },
    witness_graph: ArtifactFile {
        rel_path: "consent.graph",
        // PINNED, in lockstep with the chain: the published `ArtifactSet` for
        // `dogtag-levelb-artifacts/1` carries this same hash as `witnessMobileSha256` (ROAX
        // 2026-07-28, in-place re-publish; see docs/ARTIFACT_PIN_RUNBOOK.md). `reconcile` treats a
        // manifest `Some` against an on-chain `None` as a CONFLICT, so this field and that one move
        // together or not at all - do not revert this to `None` without deprecating the on-chain pin.
        sha256: Some(LEVEL_B_V1_WITNESS_GRAPH_SHA256),
    },
    vk: VerifyingKeyIdentity {
        verification_key_json: ArtifactFile {
            rel_path: "consent_verification_key.json",
            sha256: Some("27879dd7c4eabb6acea4d1be1249ba3c4212f95a27237e7e1e1220557b4e2d7f"),
        },
    },
};

/// Every version this build can prove: the owner-hidden consent set.
pub const REGISTRY: &[&ArtifactDescriptor] = &[&LEVEL_B_V1_DESCRIPTOR];

/// The version a caller gets when it names none — the current consent artifact set.
pub fn current() -> &'static ArtifactDescriptor {
    &LEVEL_B_V1_DESCRIPTOR
}

/// Resolve a version key to its artifact set.
///
/// * `None` → [`current`].
/// * `Some(known)` → that version's descriptor.
/// * `Some(unknown)` → [`crate::ProverError::UnknownVersion`] — **fail closed**. An unknown version
///   never falls back to the current one: proving a v2 request with v1's key would produce a proof
///   the v2 verifier rejects, and silently answering the wrong question is worse than refusing
///   (M7 §7.4 — an unseen version is a hard stop).
pub fn resolve(version: Option<&str>) -> Result<&'static ArtifactDescriptor, crate::ProverError> {
    let Some(version) = version else {
        return Ok(current());
    };
    REGISTRY
        .iter()
        .copied()
        .find(|d| d.version == version)
        .ok_or_else(|| crate::ProverError::UnknownVersion {
            version: version.to_string(),
            known: REGISTRY.iter().map(|d| d.version.to_string()).collect(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No version named ⇒ the consent set — the sole registered version is the default.
    #[test]
    fn resolve_none_is_the_consent_set() {
        assert_eq!(resolve(None).unwrap().version, LEVEL_B_V1);
        assert_eq!(current().version, LEVEL_B_V1);
    }

    /// The consent version is a registered, resolvable entry.
    #[test]
    fn resolve_consent_version_returns_its_descriptor() {
        let d = resolve(Some(LEVEL_B_V1)).unwrap();
        assert_eq!(d.version, LEVEL_B_V1);
        assert_eq!(d.zkey.rel_path, "consent_final.zkey");
        assert_eq!(d.circuit_id, "consent.circom/DogTagConsent(6)");
        assert_eq!(
            d.public_signal_layout,
            ["dogTagId", "purpose", "relayer", "nullifier", "R", "recordType", "deadline"],
            "frozen seven-OUTPUT order (consent.circom, README.consent.md)"
        );
    }

    /// An unknown (including retired) version FAILS CLOSED — it must never silently resolve to the
    /// current artifact set. `dogtag-levela/1` is the retired key; this build no longer serves it.
    #[test]
    fn resolve_unknown_version_fails_closed() {
        for unknown in ["dogtag-levela/1", "dogtag-levelc/9"] {
            match resolve(Some(unknown)) {
                Err(crate::ProverError::UnknownVersion { version, known }) => {
                    assert_eq!(version, unknown);
                    assert_eq!(known, vec![LEVEL_B_V1.to_string()]);
                }
                other => panic!("unknown version must fail closed, got {other:?}"),
            }
        }
    }

    /// The zkey pin and the VK identity are DIFFERENT values (ZK cross-check §2: `zkeySha256 !=
    /// vkHash`). A refactor that made one the other would pass every other test here.
    #[test]
    fn zkey_pin_is_not_the_vk_hash() {
        let vk = LEVEL_B_V1_DESCRIPTOR.vk.verification_key_json.sha256;
        assert!(vk.is_some(), "the consent version must publish its VK identity");
        assert_ne!(
            Some(LEVEL_B_V1_DESCRIPTOR.zkey.sha256),
            vk,
            "the zkey file hash and the VK hash are different artifacts and must never be conflated"
        );
    }

    /// Registered versions are uniquely keyed and internally consistent — `resolve` returns the first
    /// match, so a duplicate key would shadow an entry.
    #[test]
    fn registry_entries_are_unique_and_self_consistent() {
        let mut seen = std::collections::HashSet::new();
        for d in REGISTRY {
            assert!(seen.insert(d.version), "duplicate version key: {}", d.version);
            assert_eq!(
                d.public_signal_layout.len(),
                d.num_public,
                "{}: public_signal_layout must name every public signal",
                d.version
            );
            assert_eq!(
                d.num_public,
                crate::NUM_PUBLIC,
                "{}: this build formats a fixed NUM_PUBLIC-wide `pub` vector, so a registered version \
                 exposing a different count would be rejected by `Prover::load_versioned`",
                d.version
            );
            assert_eq!(
                resolve(Some(d.version)).unwrap().version,
                d.version,
                "{}: registered but not resolvable",
                d.version
            );
        }
    }

    /// `circuits/build/`, relative to this crate.
    fn build_dir() -> std::path::PathBuf {
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../circuits/build")
    }

    /// The committed witness graph IS the attested one.
    ///
    /// This is the provenance check the audit (M9 rec 10) asked for. Before the graph was committed
    /// it was an out-of-band local build that differed per machine, so nothing anywhere attested
    /// which graph an app had proved with. Committing it fixes the bytes; THIS test is what makes
    /// that a checked fact rather than an assumption, and what turns a silent regraph into a red
    /// test.
    ///
    /// Absence is a FAILURE, not a skip: the graph is committed, so a checkout without it is
    /// incomplete, and skipping would restore exactly the "green without checking" hole the
    /// consent-parity wrapper exists to close.
    #[test]
    fn graph_file_matches_attested_sha256() {
        use sha2::{Digest, Sha256};

        let path = build_dir().join(LEVEL_B_V1_DESCRIPTOR.witness_graph.rel_path);
        let bytes = std::fs::read(&path).unwrap_or_else(|e| {
            panic!(
                "consent.graph is COMMITTED under circuits/build - a checkout without it is \
                 incomplete, not a normal local state. Failed to read {}: {e}",
                path.display()
            )
        });
        let got = hex::encode::<[u8; 32]>(Sha256::digest(&bytes).into());
        assert_eq!(
            got,
            LEVEL_B_V1_WITNESS_GRAPH_SHA256,
            "committed consent.graph does not match its attested hash. If the graph was rebuilt on \
             purpose, the on-chain witnessMobileSha256 and this constant must be rotated TOGETHER - \
             see docs/ARTIFACT_PIN_RUNBOOK.md."
        );
    }

    /// The descriptor's graph pin must be the attested hash.
    ///
    /// The pin is now `Some` (flipped 2026-07-28 in lockstep with the on-chain `witnessMobileSha256`;
    /// see [`LEVEL_B_V1_WITNESS_GRAPH_SHA256`]), so the `Some` arm is the live one and it is what
    /// catches a stale or hand-typed hash here rather than as a reconcile conflict in the field. The
    /// `None` arm is retained deliberately: it is the state a future rotation passes back through, and
    /// it still asserts the attested hash stays publishable, so unpinning cannot quietly become
    /// "nobody ever attested it".
    #[test]
    fn descriptor_graph_pin_agrees_with_the_file() {
        match LEVEL_B_V1_DESCRIPTOR.witness_graph.sha256 {
            None => {
                // Unpinned is the current, intended state — assert the reason still holds, so this
                // arm cannot quietly become "nobody ever pinned it".
                assert_eq!(
                    LEVEL_B_V1_WITNESS_GRAPH_SHA256.len(),
                    64,
                    "the attested hash must remain a full SHA-256 the operator can publish"
                );
            }
            Some(pin) => assert_eq!(
                pin, LEVEL_B_V1_WITNESS_GRAPH_SHA256,
                "the descriptor pins a different graph than the committed/attested one"
            ),
        }
    }
}

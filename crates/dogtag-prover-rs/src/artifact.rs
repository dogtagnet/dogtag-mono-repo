//! Version-keyed proving artifacts (M7 §3.2).
//!
//! A protocol **version key** (e.g. [`LEVEL_A_V1`]) resolves to an [`ArtifactDescriptor`]: the set of
//! files a prover needs for that version, plus the integrity pins to check them against. This replaces
//! the single hard-coded artifact set — the pinned zkey filename + the one `EXPECTED_ZKEY_SHA256_HEX`
//! — with a table that can hold many versions.
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
//!   what [`crate::Prover::load`] hashes the bytes against BEFORE parsing them, so a swapped or
//!   corrupt key fails closed (audit M4).
//! * [`VerifyingKeyIdentity`] — which **VK** the resulting proof verifies against. The authoritative
//!   VK is the one embedded in the on-chain `Groth16Verifier`, identified by its address; the
//!   `verification_key.json` hash pinned here identifies the same VK off-chain. It is NOT a fetch pin
//!   and the prover never loads that file (it uses the VK inside the zkey).
//!
//! Conflating them would let a version publish one hash and imply the other, so they never share a
//! field.
//!
//! # Adding a version
//!
//! Add an [`ArtifactDescriptor`] const and register it in [`REGISTRY`]. The Level-B consent version
//! ([`LEVEL_B_V1_DESCRIPTOR`]) landed with the M7 P0 consent proving path: its code path is
//! [`crate::ConsentProveInputs`] / [`crate::Prover::prove_consent_inputs`] (distinct signal names
//! from Level-A's [`crate::Prover::prove`]), fed by the SDK's `consent_assemble` assembler. It is
//! built from `circuits/consent.circom` (NOT the stale Level-A `consent.rs`, ZK cross-check §2) and
//! pins its own zkey AND its own VK, separately.

/// The Level-A verification circuit, version 1 - the default (unnamed-version) protocol version.
///
/// This is what every caller that names no version resolves to ([`resolve`]), so the pre-M7 path is
/// simply this entry.
pub const LEVEL_A_V1: &str = "dogtag-levela/1";

/// The Level-B owner-unlinkable consent circuit, version 1 (M7 P0).
///
/// Its proofs verify against the frozen `Groth16VerifierConsent` VK; its inputs are the
/// [`crate::ConsentProveInputs`] shape (NOT Level-A's `ProveInputs`), and its public-signal vector is
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
/// The seed entry is [`LEVEL_A_V1_DESCRIPTOR`]. Resolve one with [`resolve`] / [`current`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArtifactDescriptor {
    /// The version key ([`LEVEL_A_V1`]) — the string callers pass over the wire.
    pub version: &'static str,
    /// The circuit this version proves, as `<source>/<template>(<params>)`.
    pub circuit_id: &'static str,
    /// How many public signals the circuit exposes.
    pub num_public: usize,
    /// The circuit's max leaf count — the `N` width its fixed-size leaf arrays carry — for versions
    /// whose inputs are the Level-A `ProveInputs` shape. `None` for a circuit that feeds no such
    /// fixed-width leaf array (the Level-B consent circuit folds depth-6 inclusion PATHS, not an
    /// `N`-wide leaf table), so the `max_leaves == N` load guard does not apply to it.
    pub max_leaves: Option<usize>,
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

/// The Level-A artifact set — the seed entry, and the default for every caller naming no version.
///
/// The zkey pin is the testnet self-run ceremony output recorded in `contracts/deployments/roax.json`
/// (`_zk_ceremony`) and `docs/CEREMONY_TRANSCRIPT.md`.
pub const LEVEL_A_V1_DESCRIPTOR: ArtifactDescriptor = ArtifactDescriptor {
    version: LEVEL_A_V1,
    circuit_id: "verification.circom/DogTagVerification(24,5)",
    num_public: crate::NUM_PUBLIC,
    max_leaves: Some(crate::N),
    public_signal_layout: &[
        "dogTagId",
        "purpose",
        "relayer",
        "subject",
        "nullifier",
        "keyHash",
        "R",
    ],
    zkey: ZkeyArtifact {
        rel_path: "verification_final.zkey",
        sha256: crate::EXPECTED_ZKEY_SHA256_HEX,
    },
    r1cs: ArtifactFile {
        rel_path: "verification.r1cs",
        sha256: Some("8890e52860b5b43535143682892bd6fa87ffefab3084b4eb2186b91c99be6fe8"),
    },
    wasm: ArtifactFile {
        rel_path: "verification_js/verification.wasm",
        sha256: Some("a2a2d7e28899e59019663f416c1ec1429ca0a35ead9363c7f832d9e476df874f"),
    },
    witness_graph: ArtifactFile {
        rel_path: "verification.graph",
        // Unpinned: unlike the zkey/r1cs/wasm, the graph is not committed — CI fetches it from
        // DOGTAG_ARTIFACTS_URL (see `.github/workflows/*-mobile-e2e.yml`), so there is no byte-stable
        // hash in-tree to pin it to. A version served by the discovery anchor publishes its graph
        // pin like any other file, and the mobile resolver checks it then.
        sha256: None,
    },
    vk: VerifyingKeyIdentity {
        verification_key_json: ArtifactFile {
            rel_path: "verification_key.json",
            sha256: Some("4a4cee60ae59dcacc27ff940c350069caca35540a75bc92fe89fa9361da9cb60"),
        },
    },
};

/// The Level-B consent artifact set (M7 P0).
///
/// The zkey/VK are the frozen M3 testnet-grade ceremony output (`docs/CEREMONY_TRANSCRIPT.consent.md`,
/// committed under `circuits/build`). `max_leaves` is `None`: `DogTagConsent(6)` feeds depth-6
/// inclusion PATHS via [`crate::ConsentProveInputs`], not an `N`-wide leaf table, so the
/// `max_leaves == N` load guard does not apply. Its public-signal layout is the frozen seven-OUTPUT
/// order; the graph is not committed (fetched in CI, like Level-A's).
pub const LEVEL_B_V1_DESCRIPTOR: ArtifactDescriptor = ArtifactDescriptor {
    version: LEVEL_B_V1,
    circuit_id: "consent.circom/DogTagConsent(6)",
    num_public: crate::NUM_PUBLIC,
    max_leaves: None,
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
        // Unpinned: the graph is not committed (CI fetches it from DOGTAG_ARTIFACTS_URL), like
        // Level-A's. The mobile resolver pins it from the discovery anchor at fetch time.
        sha256: None,
    },
    vk: VerifyingKeyIdentity {
        verification_key_json: ArtifactFile {
            rel_path: "consent_verification_key.json",
            sha256: Some("27879dd7c4eabb6acea4d1be1249ba3c4212f95a27237e7e1e1220557b4e2d7f"),
        },
    },
};

/// Every version this build can prove: Level-A verification + Level-B consent (M7 P0).
pub const REGISTRY: &[&ArtifactDescriptor] =
    &[&LEVEL_A_V1_DESCRIPTOR, &LEVEL_B_V1_DESCRIPTOR];

/// The version a caller gets when it names none — the current Level-A artifact set.
pub fn current() -> &'static ArtifactDescriptor {
    &LEVEL_A_V1_DESCRIPTOR
}

/// Resolve a version key to its artifact set.
///
/// * `None` → [`current`] (back-compat: every pre-M7 caller names no version).
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

    /// No version named ⇒ the Level-A set. This is the back-compat contract every pre-M7 caller
    /// relies on.
    #[test]
    fn resolve_none_is_level_a() {
        assert_eq!(resolve(None).unwrap().version, LEVEL_A_V1);
        assert_eq!(current().version, LEVEL_A_V1);
    }

    #[test]
    fn resolve_known_version_returns_its_descriptor() {
        let d = resolve(Some(LEVEL_A_V1)).unwrap();
        assert_eq!(d.version, LEVEL_A_V1);
        assert_eq!(d.zkey.rel_path, "verification_final.zkey");
    }

    /// The Level-B consent version (M7 P0) is now a registered, resolvable entry.
    #[test]
    fn resolve_consent_version_returns_its_descriptor() {
        let d = resolve(Some(LEVEL_B_V1)).unwrap();
        assert_eq!(d.version, LEVEL_B_V1);
        assert_eq!(d.zkey.rel_path, "consent_final.zkey");
        assert_eq!(d.circuit_id, "consent.circom/DogTagConsent(6)");
        assert_eq!(d.max_leaves, None, "consent feeds inclusion paths, not an N-wide leaf array");
        assert_eq!(
            d.public_signal_layout,
            ["dogTagId", "purpose", "relayer", "nullifier", "R", "recordType", "deadline"],
            "frozen seven-OUTPUT order (consent.circom, README.consent.md)"
        );
    }

    /// An unknown version FAILS CLOSED — it must never silently resolve to the current artifact set.
    #[test]
    fn resolve_unknown_version_fails_closed() {
        match resolve(Some("dogtag-levelc/9")) {
            Err(crate::ProverError::UnknownVersion { version, known }) => {
                assert_eq!(version, "dogtag-levelc/9");
                assert_eq!(known, vec![LEVEL_A_V1.to_string(), LEVEL_B_V1.to_string()]);
            }
            other => panic!("unknown version must fail closed, got {other:?}"),
        }
    }

    /// The seed descriptor's zkey pin IS the crate's published constant — one pin, two names, never
    /// allowed to drift apart.
    #[test]
    fn level_a_zkey_pin_matches_crate_constant() {
        assert_eq!(
            LEVEL_A_V1_DESCRIPTOR.zkey.sha256,
            crate::EXPECTED_ZKEY_SHA256_HEX
        );
    }

    /// The zkey pin and the VK identity are DIFFERENT values (ZK cross-check §2: `zkeySha256 !=
    /// vkHash`). A refactor that made one the other would pass every other test here.
    #[test]
    fn zkey_pin_is_not_the_vk_hash() {
        let vk = LEVEL_A_V1_DESCRIPTOR.vk.verification_key_json.sha256;
        assert!(vk.is_some(), "Level-A must publish its VK identity");
        assert_ne!(
            Some(LEVEL_A_V1_DESCRIPTOR.zkey.sha256),
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
            // A version that DECLARES a fixed leaf-array width must match this build's N (the guard
            // `Prover::load_versioned` enforces). A version with `None` (the consent circuit) feeds
            // inclusion paths, not a fixed leaf table, so it is exempt.
            if let Some(max_leaves) = d.max_leaves {
                assert_eq!(
                    max_leaves,
                    crate::N,
                    "{}: a fixed-width leaf-array version must feed exactly N; \
                     `Prover::load_versioned` would reject any other width",
                    d.version
                );
            }
            assert_eq!(
                resolve(Some(d.version)).unwrap().version,
                d.version,
                "{}: registered but not resolvable",
                d.version
            );
        }
    }
}

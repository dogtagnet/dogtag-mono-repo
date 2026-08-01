//! The content-addressed mirror (registry-plan S-17).
//!
//! `ProviderDirectory.ProfileAnchor` publishes a `digest` and says nothing about where the bytes
//! live. This module is the "where": a store keyed by the content address itself, so a reader can
//! ask for an address and check what comes back against the address it asked under.
//!
//! # The address IS the hash, so location confers nothing
//!
//! Serving is not evidence. A consumer MUST recompute the digest over the bytes it received and
//! compare it to the address it requested; that recomputation is the security boundary and it lives
//! in the client (`packages/ui/src/mirror/contentAddress.ts`). The ingest check below is DEFENCE IN
//! DEPTH: it stops the mirror from ever holding bytes under an address they do not hash to, which
//! makes a mismatch on read a transport or storage fault rather than a routine occurrence. It is
//! deliberately NOT a licence for a client to skip its own check — a client that trusts the mirror
//! because "the mirror already checked" has turned content addressing back into trusting where the
//! bytes came from, which is the one thing content addressing exists to remove.
//!
//! The read path re-verifies too, for the same reason and with the same standing: a stored blob that
//! no longer hashes to its key is corruption, and answering `Corrupt` is how the mirror declines to
//! serve it. It must never be reported as "not found" — those have different remedies (re-publish
//! versus investigate the store), and this repo's standing rule is that a could-not-serve may not be
//! rendered as its benign neighbour.
//!
//! # Publication-safe content only
//!
//! Anyone holding an address can fetch what it names. Content addressing gives integrity, never
//! privacy — the same sentence `ProviderDirectory.setProfileAnchor` carries. Nothing private may be
//! mirrored.
//!
//! # BOUNDED, AND IT FILLS RATHER THAN EVICTING
//!
//! The write surface is reachable by whoever holds the `MIRROR_INGEST_TOKEN` - and that token is
//! PUBLIC BY CONSTRUCTION wherever a portal publishes, because vite inlines it into the shipped
//! bundle as `VITE_CONTENT_MIRROR_TOKEN`. So the population that can fill this store is EVERY
//! VISITOR TO EITHER PORTAL, which is strictly wider than the third-party deployments holding scope
//! tokens that an earlier draft of this paragraph named. Read that as the caps mattering MORE since
//! the write gate moved to a dedicated token, never less: narrowing what the token GRANTS did not
//! narrow who HOLDS it, and these caps are now the only thing bounding public writes to the service
//! that also serves the government oversight feed.
//! [`MAX_CONTENT_BYTES`] bounds one object and bounds nothing about accumulation, so
//! [`MAX_MIRROR_OBJECTS`] and [`MAX_MIRROR_TOTAL_BYTES`] bound the store. BOTH, because a million
//! one-byte objects and a smaller set of 512 KiB ones are different exhaustion routes and neither
//! cap bounds the other.
//!
//! **THERE IS NO EVICTION, and that is a deferral rather than an omission.** At either cap the
//! mirror REFUSES new content - loudly, with [`IngestRejection::MirrorFull`] naming the limit it hit
//! and a 507 from the route - and it never drops, overwrites or ages out anything to make room. An
//! operator's remedy today is a restart, which empties an already-ephemeral store. LRU/TTL eviction
//! is the follow-up, and it is not free to design: this module answers 404 for "never published, or
//! withdrawn", so evicting a live object silently converts a published listing into that state.
//! Nothing here should be read as implying eviction exists.
//!
//! A re-publish of content the mirror ALREADY holds is not growth and does not count against either
//! cap. That is load-bearing rather than generous: re-publishing is the normal repair path after a
//! restart, so a cap that counted it would refuse exactly the operation that restores a lost object.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use alloy::primitives::keccak256;
use async_trait::async_trait;

/// Multicodec code for keccak-256, matching `ProfileAnchor.hashAlgorithm` as written by
/// `packages/ui/src/provider/directoryPlan.ts` (`MULTIHASH_KECCAK_256`).
///
/// The mirror supports exactly this one, and an address is always a keccak-256 digest. A second
/// algorithm would need its own address namespace: two functions can produce the same 32 bytes for
/// different inputs, so one flat keyspace across algorithms would let a blob published under one
/// verify under the other.
pub const MULTIHASH_KECCAK_256: u8 = 0x1b;

/// The largest object the mirror accepts.
///
/// A logo, not a photo library. Bounded because ingest holds the whole body in memory to hash it,
/// and because an unbounded public read surface is a bandwidth amplifier. 512 KiB is generous for a
/// raster logo and small enough that a full page of them is affordable.
pub const MAX_CONTENT_BYTES: usize = 512 * 1024;

/// How many distinct objects the mirror will hold.
///
/// A provider publishes at most two - its profile document and its logo - so this is roughly 2,048
/// providers. State that plainly rather than as a capacity claim: the production directory is sized
/// at hundreds of thousands of providers, so this in-memory store is a development and testnet
/// store that FILLS, not a production one. A durable store with eviction slots in behind
/// [`ContentMirror`] and is the follow-up; see the module doc.
pub const MAX_MIRROR_OBJECTS: usize = 4_096;

/// How many bytes the mirror will hold across every object.
///
/// Separate from [`MAX_MIRROR_OBJECTS`] because neither bounds the other: at [`MAX_CONTENT_BYTES`]
/// each, the object cap alone would permit 2 GiB, and at one byte each the byte cap alone would
/// permit 134 million objects. Whichever binds first is the one that refuses.
pub const MAX_MIRROR_TOTAL_BYTES: usize = 128 * 1024 * 1024;

/// Which cap an ingest hit. Carried on the rejection so the message names the real constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MirrorLimit {
    Objects,
    TotalBytes,
}

#[derive(Debug, thiserror::Error)]
pub enum MirrorError {
    /// The store itself could not answer. NEVER conflated with "no such address": one is a fact
    /// about the mirror's contents, the other is a failure to establish anything.
    #[error("{0}")]
    Unavailable(String),
}

/// What a read of one address established.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContentRead {
    /// The bytes, which re-hashed to the requested address on the way out.
    Found { bytes: Vec<u8>, media_type: String },
    /// The mirror holds nothing at this address. A fact, and the honest answer for content that was
    /// never published or has been withdrawn.
    Absent,
    /// The mirror holds bytes here and they no longer hash to the address. Storage corruption, and
    /// the one case where the mirror refuses to serve what it has.
    Corrupt,
}

/// Why an ingest was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestRejection {
    /// The submitted bytes do not hash to the address they were submitted under. The whole point.
    AddressMismatch { computed: String },
    /// Larger than [`MAX_CONTENT_BYTES`].
    TooLarge { len: usize },
    /// The address is not a lowercase `0x` + 64 hex word.
    MalformedAddress,
    /// A media type this mirror will not serve. See [`is_servable_media_type`].
    UnsupportedMediaType { media_type: String },
    /// The declared media type disagrees with what the bytes actually are.
    ///
    /// A correct hash proves the bytes are the ones that were published; it proves nothing about
    /// what they ARE. The declaration is provider-supplied, so it is checked against the bytes
    /// rather than believed.
    MediaTypeMismatch { declared: String, sniffed: String },
    /// The mirror is at a capacity cap. The request is WELL FORMED and the mirror is full, which is
    /// why the route answers 507 rather than 400: nothing about the content is wrong.
    MirrorFull {
        limit: MirrorLimit,
        cap: usize,
        held: usize,
    },
}

impl IngestRejection {
    pub fn message(&self) -> String {
        match self {
            Self::AddressMismatch { computed } => {
                format!("content does not hash to the address it was submitted under (computed {computed})")
            }
            Self::TooLarge { len } => {
                format!("content is {len} bytes, over the {MAX_CONTENT_BYTES}-byte limit")
            }
            Self::MalformedAddress => {
                "address must be a lowercase 0x-prefixed 32-byte hex word".to_string()
            }
            Self::UnsupportedMediaType { media_type } => {
                format!("this mirror does not serve {media_type}")
            }
            Self::MediaTypeMismatch { declared, sniffed } => {
                format!("declared {declared} but the bytes are {sniffed}")
            }
            Self::MirrorFull {
                limit: MirrorLimit::Objects,
                cap,
                held,
            } => format!(
                "the content mirror is full: it holds {held} objects and will hold {cap}. It does not evict, so nothing was displaced to make room"
            ),
            Self::MirrorFull {
                limit: MirrorLimit::TotalBytes,
                cap,
                held,
            } => format!(
                "the content mirror is full: it holds {held} bytes and will hold {cap}. It does not evict, so nothing was displaced to make room"
            ),
        }
    }
}

/// Whether one more object fits, given what the store already holds.
///
/// Pure, so the whole capacity ladder is unit-testable without a store, exactly like
/// [`check_ingest`]. The store-state read that feeds it happens under the lock in the
/// implementation, so the decision cannot race the insert it guards.
///
/// `replacing` is the size of what is ALREADY stored at this address, when anything is. Content
/// addressing makes that delta zero by construction - the same address is the same bytes is the
/// same length - so the subtraction is defensive rather than load-bearing, and it must not be
/// "simplified" into an unconditional `+ incoming`: what matters is that an already-held address
/// counts as NO growth at all, which is what keeps the re-publish repair path open at the cap.
pub fn check_capacity(
    held_objects: usize,
    held_bytes: usize,
    replacing: Option<usize>,
    incoming: usize,
) -> Result<(), IngestRejection> {
    let objects_after = held_objects + usize::from(replacing.is_none());
    if objects_after > MAX_MIRROR_OBJECTS {
        return Err(IngestRejection::MirrorFull {
            limit: MirrorLimit::Objects,
            cap: MAX_MIRROR_OBJECTS,
            held: held_objects,
        });
    }
    let bytes_after = held_bytes - replacing.unwrap_or(0) + incoming;
    if bytes_after > MAX_MIRROR_TOTAL_BYTES {
        return Err(IngestRejection::MirrorFull {
            limit: MirrorLimit::TotalBytes,
            cap: MAX_MIRROR_TOTAL_BYTES,
            held: held_bytes,
        });
    }
    Ok(())
}

/// The content address of `bytes` — lowercase `0x` + 64 hex.
pub fn content_address(bytes: &[u8]) -> String {
    format!("0x{}", hex::encode(keccak256(bytes)))
}

/// Normalize a caller-supplied address, or `None` when it is not a 32-byte hex word.
///
/// Case-insensitive on the way in (a caller formatting a bytes32 as uppercase hex is an ordinary
/// wire difference, not an attack) and lowercase on the way out, so the keyspace has one spelling.
pub fn normalize_address(raw: &str) -> Option<String> {
    let body = raw.strip_prefix("0x").or_else(|| raw.strip_prefix("0X"))?;
    if body.len() != 64 || !body.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("0x{}", body.to_ascii_lowercase()))
}

/// The image types this mirror will store and serve.
///
/// **SVG is deliberately absent, and that is a security decision rather than an omission.** SVG is
/// an XML document that can carry `<script>`, and these bytes are rendered inside operator portals
/// holding an admin session. A correct content address proves the bytes are the ones the provider
/// published — it says nothing about whether they are safe to render, and a provider is exactly the
/// party whose claim the whole slice declines to take on trust. Raster only.
///
/// **HAND-MIRRORED in TypeScript as `SERVABLE_IMAGE_MEDIA_TYPES` in
/// `packages/ui/src/mirror/profileBlob.ts`, and the two move together or not at all.** Nothing
/// derives one from the other, so the guard is the one this repo gives its other cross-language
/// folds: each side pins its own list exhaustively (here,
/// `the_servable_image_set_is_closed_and_hand_mirrored_in_typescript`) so an accidental edit reddens,
/// plus this note, because neither test can read the other language.
///
/// Mirror the IMAGE list ONLY. [`PROFILE_MEDIA_TYPE`] is servable too, being the profile blob's own
/// type, but it must never join the TypeScript list: that list is the allowlist a LOGO entry is
/// checked against, and a logo declaring `application/json` would name a document a portal then
/// hands to an `<img>`.
///
/// The two drift directions fail differently, and only one of them is quiet:
///
/// * **This side widens alone** - the mirror accepts a logo media type `parseProfileBlob` refuses,
///   and a refused logo entry fails the WHOLE profile document by design, so that provider's
///   CONTACTS are withheld as well and nothing anywhere reports why. This is the dangerous
///   direction, and it is why the pin lives here rather than only in TypeScript.
/// * **TypeScript widens alone** - a publication names a type [`check_ingest`] refuses, so the
///   `mirrorUpload` step fails and publication is blocked loudly rather than silently.
pub const SERVABLE_IMAGE_MEDIA_TYPES: &[&str] =
    &["image/png", "image/jpeg", "image/webp", "image/gif"];

/// Whether this mirror will store and serve `media_type`: an image from
/// [`SERVABLE_IMAGE_MEDIA_TYPES`], or the profile blob's own [`PROFILE_MEDIA_TYPE`].
pub fn is_servable_media_type(media_type: &str) -> bool {
    SERVABLE_IMAGE_MEDIA_TYPES.contains(&media_type) || media_type == PROFILE_MEDIA_TYPE
}

/// The profile blob's own media type. JSON, and inert.
pub const PROFILE_MEDIA_TYPE: &str = "application/json";

/// What the bytes actually are, by magic number, or `None` when unrecognised.
///
/// Only the types [`is_servable_media_type`] admits are recognised, so an unrecognised result is
/// itself a refusal rather than a shrug.
pub fn sniff_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Some("image/jpeg");
    }
    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return Some("image/gif");
    }
    // JSON has no magic number, so it is recognised structurally: the profile blob is an object.
    // Leading whitespace is permitted because a serializer may emit it; anything else is not JSON
    // this mirror will claim to serve.
    let trimmed = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|i| &bytes[i..]);
    if trimmed.is_some_and(|t| t.starts_with(b"{")) && serde_json::from_slice::<serde_json::Value>(bytes).is_ok() {
        return Some(PROFILE_MEDIA_TYPE);
    }
    None
}

/// Validate an ingest without touching a store. Pure, so the whole refusal ladder is unit-testable.
///
/// Returns the normalized address on acceptance.
pub fn check_ingest(
    raw_address: &str,
    bytes: &[u8],
    declared_media_type: &str,
) -> Result<String, IngestRejection> {
    let address = normalize_address(raw_address).ok_or(IngestRejection::MalformedAddress)?;
    if bytes.len() > MAX_CONTENT_BYTES {
        return Err(IngestRejection::TooLarge { len: bytes.len() });
    }
    if !is_servable_media_type(declared_media_type) {
        return Err(IngestRejection::UnsupportedMediaType {
            media_type: declared_media_type.to_string(),
        });
    }
    match sniff_media_type(bytes) {
        Some(sniffed) if sniffed == declared_media_type => {}
        Some(sniffed) => {
            return Err(IngestRejection::MediaTypeMismatch {
                declared: declared_media_type.to_string(),
                sniffed: sniffed.to_string(),
            })
        }
        None => {
            return Err(IngestRejection::MediaTypeMismatch {
                declared: declared_media_type.to_string(),
                sniffed: "unrecognised".to_string(),
            })
        }
    }
    // Last, because it is the expensive one and because a mismatch here is the headline refusal:
    // reporting "wrong media type" for content that is also at the wrong address would send the
    // publisher after the smaller of two problems.
    let computed = content_address(bytes);
    if computed != address {
        return Err(IngestRejection::AddressMismatch { computed });
    }
    Ok(address)
}

/// The mirror's storage surface.
#[async_trait]
pub trait ContentMirror: Send + Sync {
    /// Read one address. Fallible on purpose — see [`MirrorError::Unavailable`].
    async fn get(&self, address: &str) -> Result<ContentRead, MirrorError>;

    /// Store `bytes` under their own content address, which is returned.
    ///
    /// Implementations MUST reject content that does not hash to `raw_address`; that check is what
    /// makes this a content-addressed store rather than a key-value store with hexadecimal keys.
    async fn put(
        &self,
        raw_address: &str,
        bytes: Vec<u8>,
        media_type: &str,
    ) -> Result<Result<String, IngestRejection>, MirrorError>;
}

// ------------------------------------------------------------------------------------------------
// MemContentMirror
// ------------------------------------------------------------------------------------------------

#[derive(Default)]
struct MemInner {
    objects: HashMap<String, (Vec<u8>, String)>,
    /// Running total, so a capacity decision is not an O(n) sum on every write. Maintained by
    /// [`MemInner::insert`] ALONE - every writer goes through it, including `plant_unchecked`,
    /// because a second insert site is how the accounting drifts away from the map it describes.
    total_bytes: usize,
    fail_reads: bool,
}

impl MemInner {
    fn held(&self, address: &str) -> Option<usize> {
        self.objects.get(address).map(|(bytes, _)| bytes.len())
    }

    fn insert(&mut self, address: String, bytes: Vec<u8>, media_type: String) {
        let incoming = bytes.len();
        let displaced = self
            .objects
            .insert(address, (bytes, media_type))
            .map_or(0, |(old, _)| old.len());
        self.total_bytes = self.total_bytes - displaced + incoming;
    }
}

/// In-memory mirror: the demo/test store, and the one the hermetic suites drive.
#[derive(Clone, Default)]
pub struct MemContentMirror {
    inner: Arc<Mutex<MemInner>>,
}

impl MemContentMirror {
    pub fn new() -> Self {
        Self::default()
    }

    /// Fault injection for the unreadable-store branch.
    ///
    /// A store that cannot fail cannot exercise the 503 path, and an untested could-not-read
    /// renderer is how a failure ships disguised as an absence.
    pub fn set_fail_reads(&self, fail: bool) {
        self.inner.lock().unwrap().fail_reads = fail;
    }

    /// Write bytes under an address WITHOUT the ingest check.
    ///
    /// Test-only, and the only way to reach [`ContentRead::Corrupt`]: `put` cannot produce that
    /// state by construction, so a corrupt row has to be planted. It is also how the "a valid but
    /// WRONG image is served under the requested address" case is staged — the discriminator that
    /// tells a working client check from a broken one, because a merely-unparseable image would be
    /// refused by the browser's decoder and the test would pass for the wrong reason.
    pub fn plant_unchecked(&self, raw_address: &str, bytes: Vec<u8>, media_type: &str) {
        let address = normalize_address(raw_address).unwrap_or_else(|| raw_address.to_string());
        self.inner
            .lock()
            .unwrap()
            .insert(address, bytes, media_type.to_string());
    }

    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Total bytes held, for the capacity tests and for an operator probe.
    pub fn total_bytes(&self) -> usize {
        self.inner.lock().unwrap().total_bytes
    }
}

#[async_trait]
impl ContentMirror for MemContentMirror {
    async fn get(&self, address: &str) -> Result<ContentRead, MirrorError> {
        let Some(address) = normalize_address(address) else {
            // A malformed address names nothing, and cannot name anything: this is a fact about the
            // request, so it is `Absent` rather than an error about the store.
            return Ok(ContentRead::Absent);
        };
        let inner = self.inner.lock().unwrap();
        if inner.fail_reads {
            return Err(MirrorError::Unavailable("content store read failed".into()));
        }
        let Some((bytes, media_type)) = inner.objects.get(&address) else {
            return Ok(ContentRead::Absent);
        };
        if content_address(bytes) != address {
            return Ok(ContentRead::Corrupt);
        }
        Ok(ContentRead::Found {
            bytes: bytes.clone(),
            media_type: media_type.clone(),
        })
    }

    async fn put(
        &self,
        raw_address: &str,
        bytes: Vec<u8>,
        media_type: &str,
    ) -> Result<Result<String, IngestRejection>, MirrorError> {
        let address = match check_ingest(raw_address, &bytes, media_type) {
            Ok(address) => address,
            Err(rejection) => return Ok(Err(rejection)),
        };
        // The capacity decision and the insert it guards happen under ONE lock, so a concurrent
        // publication cannot slip between them and carry the store past a cap.
        let mut inner = self.inner.lock().unwrap();
        if let Err(rejection) = check_capacity(
            inner.objects.len(),
            inner.total_bytes,
            inner.held(&address),
            bytes.len(),
        ) {
            return Ok(Err(rejection));
        }
        inner.insert(address.clone(), bytes, media_type.to_string());
        Ok(Ok(address))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        let mut v = b"\x89PNG\r\n\x1a\n".to_vec();
        v.extend_from_slice(b"a-plausible-png-body");
        v
    }

    #[test]
    fn the_address_is_the_hash_of_the_bytes() {
        let a = content_address(b"hello");
        assert_eq!(a.len(), 66);
        assert!(a.starts_with("0x"));
        assert_eq!(a, content_address(b"hello"));
        assert_ne!(a, content_address(b"hellp"));
    }

    #[test]
    fn addresses_normalize_to_one_spelling_and_malformed_ones_are_refused() {
        let lower = content_address(b"x");
        let upper = format!("0x{}", lower[2..].to_ascii_uppercase());
        assert_eq!(normalize_address(&upper).as_deref(), Some(lower.as_str()));
        assert_eq!(normalize_address("0x12"), None);
        assert_eq!(normalize_address(&lower[2..]), None, "a bare hex body is not an address");
        assert_eq!(normalize_address(&format!("0x{}", "z".repeat(64))), None);
    }

    #[tokio::test]
    async fn content_submitted_under_the_wrong_address_is_refused() {
        let mirror = MemContentMirror::new();
        let bytes = png();
        let wrong = content_address(b"something else entirely");

        let rejection = mirror
            .put(&wrong, bytes.clone(), "image/png")
            .await
            .expect("store is available")
            .expect_err("must refuse");
        assert!(matches!(rejection, IngestRejection::AddressMismatch { .. }));
        assert!(mirror.is_empty(), "nothing may be stored by a refused ingest");
    }

    #[tokio::test]
    async fn accepted_content_round_trips_under_its_own_address() {
        let mirror = MemContentMirror::new();
        let bytes = png();
        let address = content_address(&bytes);

        let stored = mirror
            .put(&address, bytes.clone(), "image/png")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored, address);
        assert_eq!(
            mirror.get(&address).await.unwrap(),
            ContentRead::Found { bytes, media_type: "image/png".into() }
        );
    }

    #[tokio::test]
    async fn an_unknown_address_is_absent_and_an_unreadable_store_is_an_error() {
        let mirror = MemContentMirror::new();
        let address = content_address(b"never published");
        assert_eq!(mirror.get(&address).await.unwrap(), ContentRead::Absent);

        mirror.set_fail_reads(true);
        assert!(
            mirror.get(&address).await.is_err(),
            "a store that could not answer must not report an absence"
        );
    }

    #[tokio::test]
    async fn a_corrupted_row_is_refused_on_read_rather_than_served() {
        let mirror = MemContentMirror::new();
        let honest = png();
        let address = content_address(&honest);
        let mut tampered = honest.clone();
        *tampered.last_mut().unwrap() ^= 0xFF;

        mirror.plant_unchecked(&address, tampered, "image/png");
        assert_eq!(mirror.get(&address).await.unwrap(), ContentRead::Corrupt);
    }

    #[test]
    fn svg_is_refused_however_correct_its_address() {
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
        let address = content_address(svg);
        let rejection = check_ingest(&address, svg, "image/svg+xml").expect_err("must refuse");
        assert!(matches!(rejection, IngestRejection::UnsupportedMediaType { .. }));
    }

    #[test]
    fn the_servable_image_set_is_closed_and_hand_mirrored_in_typescript() {
        // Pinned as a CLOSED set rather than by naming a few members, because the drift that matters
        // is a widening nobody thinks is dangerous. `svg_is_refused_however_correct_its_address`
        // catches the one type that is obviously unsafe; an ordinary new raster format added here
        // reddens nothing without this, and `packages/ui/src/mirror/profileBlob.ts` would then refuse
        // a logo the mirror had accepted - which fails the whole profile document and withholds that
        // provider's contacts too. Widening this list means widening SERVABLE_IMAGE_MEDIA_TYPES
        // there in the same change; see the note on the constant.
        assert_eq!(
            SERVABLE_IMAGE_MEDIA_TYPES,
            ["image/png", "image/jpeg", "image/webp", "image/gif"],
        );
        for media_type in SERVABLE_IMAGE_MEDIA_TYPES {
            assert!(is_servable_media_type(media_type), "{media_type} must stay servable");
        }
        // Servable, and deliberately NOT part of the image half: a logo entry may never declare it.
        assert!(is_servable_media_type(PROFILE_MEDIA_TYPE));
        assert!(!SERVABLE_IMAGE_MEDIA_TYPES.contains(&PROFILE_MEDIA_TYPE));
    }

    #[test]
    fn a_media_type_that_disagrees_with_the_bytes_is_refused() {
        let bytes = png();
        let address = content_address(&bytes);
        let rejection =
            check_ingest(&address, &bytes, "image/jpeg").expect_err("declared type must be checked");
        assert!(matches!(rejection, IngestRejection::MediaTypeMismatch { .. }));
    }

    #[test]
    fn an_svg_relabelled_as_a_png_is_still_refused() {
        // The sniff is what stops the allowlist being bypassed by a lie about the type.
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><script>alert(1)</script></svg>"#;
        let address = content_address(svg);
        let rejection = check_ingest(&address, svg, "image/png").expect_err("must refuse");
        assert!(matches!(rejection, IngestRejection::MediaTypeMismatch { .. }));
    }

    #[test]
    fn the_profile_blob_is_servable_and_recognised_as_json() {
        let blob = br#"{"schema":"dogtag/provider-profile/2","contact":{}}"#;
        let address = content_address(blob);
        assert_eq!(check_ingest(&address, blob, PROFILE_MEDIA_TYPE).unwrap(), address);
    }

    #[test]
    fn the_per_object_cap_is_hand_mirrored_in_typescript() {
        // The TypeScript half is `MAX_CONTENT_BYTES` in `packages/ui/src/mirror/profileBlob.ts`,
        // which refuses an oversized logo in the picker with a visible reason rather than letting
        // the provider discover it as a 413 after a file upload. Same treatment as
        // SERVABLE_IMAGE_MEDIA_TYPES: no shared source, so each side pins its own value and the
        // notes at both constants carry the pair.
        assert_eq!(MAX_CONTENT_BYTES, 512 * 1024);
    }

    #[test]
    fn the_mirror_refuses_at_the_object_cap_rather_than_evicting() {
        assert!(matches!(
            check_capacity(MAX_MIRROR_OBJECTS, 0, None, 1),
            Err(IngestRejection::MirrorFull { limit: MirrorLimit::Objects, .. })
        ));
        // One below the cap still admits, so the boundary is the cap and not one short of it.
        assert!(check_capacity(MAX_MIRROR_OBJECTS - 1, 0, None, 1).is_ok());
    }

    #[test]
    fn the_mirror_refuses_at_the_total_byte_cap_too_because_neither_cap_bounds_the_other() {
        // Well under the OBJECT cap and still refused: a small number of large objects is its own
        // exhaustion route, which is why both caps exist.
        assert!(matches!(
            check_capacity(2, MAX_MIRROR_TOTAL_BYTES, None, 1),
            Err(IngestRejection::MirrorFull { limit: MirrorLimit::TotalBytes, .. })
        ));
        assert!(check_capacity(2, MAX_MIRROR_TOTAL_BYTES - 1, None, 1).is_ok());
    }

    #[test]
    fn re_publishing_content_already_held_is_admitted_at_both_caps() {
        // The repair path after a restart, and the reason the delta is computed against what is
        // already stored rather than assumed to be growth. A cap that counted a re-publish would
        // refuse exactly the operation that restores a lost object.
        assert!(check_capacity(MAX_MIRROR_OBJECTS, MAX_MIRROR_TOTAL_BYTES, Some(64), 64).is_ok());
    }

    #[tokio::test]
    async fn a_refused_ingest_stores_nothing_and_leaves_the_accounting_alone() {
        let mirror = MemContentMirror::new();
        let honest = png();
        let address = content_address(&honest);
        mirror.put(&address, honest.clone(), "image/png").await.unwrap().unwrap();
        assert_eq!(mirror.len(), 1);
        assert_eq!(mirror.total_bytes(), honest.len());

        let other = b"\x89PNG\r\n\x1a\n a different body".to_vec();
        let rejected = mirror
            .put(&content_address(&other), other, "image/jpeg")
            .await
            .unwrap()
            .expect_err("declared type disagrees with the bytes");
        assert!(matches!(rejected, IngestRejection::MediaTypeMismatch { .. }));
        assert_eq!(mirror.len(), 1, "a refused ingest may store nothing");
        assert_eq!(mirror.total_bytes(), honest.len(), "nor move the accounting");
    }

    #[tokio::test]
    async fn re_publishing_the_same_bytes_does_not_double_count_them() {
        let mirror = MemContentMirror::new();
        let bytes = png();
        let address = content_address(&bytes);
        for _ in 0..3 {
            mirror.put(&address, bytes.clone(), "image/png").await.unwrap().unwrap();
        }
        assert_eq!(mirror.len(), 1);
        assert_eq!(mirror.total_bytes(), bytes.len());
    }

    #[test]
    fn oversized_content_is_refused_before_it_is_hashed_into_the_store() {
        let bytes = vec![0u8; MAX_CONTENT_BYTES + 1];
        let address = content_address(&bytes);
        assert!(matches!(
            check_ingest(&address, &bytes, "image/png"),
            Err(IngestRejection::TooLarge { .. })
        ));
    }
}

#!/bin/bash
# registry-plan S-17: the repeatable mutation gate for the content-addressed mirror.
#
# Run it to prove the slice's claims are PINNED rather than merely present. Same discipline as
# scripts/verify-provider-selfservice-mutations.sh, and for the same reasons:
#
#   * bash, not zsh - zsh does not word-split an unquoted "$var", so a list-driven loop there runs
#     once with the whole string and every search misses, which is a check that passes by not
#     running;
#   * LC_ALL=C exported for the WHOLE script, so the verdict does not depend on the caller's locale;
#   * a mutation that leaves the suite GREEN is this harness's own failure, and one whose search
#     string no longer matches is reported INERT rather than counted as evidence. An unapplied
#     mutation produces a false "pinned" reading, which is the exact mistake
#     scripts/verify-issuance-authority-mutations.sh exists to stop making. This harness has already
#     caught one: `if (!parsed.ok)` -> `if (false)` was GREEN on the first attempt, because no test
#     reached a blob that VERIFIED but could not be PARSED. The coverage was added rather than the
#     mutation dropped.
#
# TWO runners, and the split is not cosmetic. `mutate` drives the TypeScript suites via vitest;
# `mutate_rs` drives the Rust mirror via cargo. The mirror's ingest and read checks live in Rust and
# cannot be reached from a vitest spec at all, so pointing `mutate` at them would match no test and
# report a green.
set -u
cd "$(cd "$(dirname "$0")/.." && pwd)"
export LC_ALL=C

TS_TARGETS="packages/ui/src/mirror/contentAddress.ts packages/ui/src/mirror/resolveProfile.ts packages/ui/src/mirror/profileBlob.ts packages/ui/src/mirror/ProviderLogo.tsx packages/ui/src/provider/directoryPlan.ts packages/ui/src/domain/ProviderSelfServicePanel.tsx"
RS_TARGETS="stacks/indexer/api/src/mirror.rs"
BACKUP=$(mktemp -d)
for f in $TS_TARGETS $RS_TARGETS; do mkdir -p "$BACKUP/$(dirname "$f")"; cp "$f" "$BACKUP/$f"; done
restore() { for f in $TS_TARGETS $RS_TARGETS; do cp "$BACKUP/$f" "$f"; done; }
trap restore EXIT

fails=0

# Returns non-zero when the search string is ABSENT, which the callers report as INERT.
#
# This is the authoritative presence check, and it replaced a `grep -qF` one that was WRONG for
# the multi-line mutations: grep -F treats embedded newlines as separate patterns and ORs them,
# so a mutation whose exact text had drifted still matched one of its lines, passed the check,
# then died inside this function - and was reported GREEN, i.e. "the claim is not pinned", when
# in truth the mutation had never been applied. That is this harness's own
# could-not-check-rendered-as-its-neighbour bug, in exactly the shape its header warns about, and
# a real drift has already hidden behind it.
apply() {
  local file="$1" old="$2" new="$3"
  python3 - "$file" "$old" "$new" <<'PY'
import sys
p, old, new = sys.argv[1], sys.argv[2], sys.argv[3]
s = open(p).read()
if old not in s:
    sys.exit(3)
open(p, "w").write(s.replace(old, new, 1))
PY
}

# $1 label, $2 file, $3 old, $4 new, $5 test file
mutate() {
  local label="$1" file="$2" old="$3" new="$4" spec="$5"
  restore
  if ! apply "$file" "$old" "$new"; then
    echo "INERT  $label -- search string absent from $file; mutation never applied"
    fails=$((fails+1)); return
  fi
  out=$(pnpm --filter @dogtag/ui exec vitest run "$spec" 2>&1)
  if echo "$out" | grep -qE "Tests +[0-9]+ failed"; then
    echo "PINNED $label -- $(echo "$out" | grep -oE "Tests +[0-9]+ failed" | head -1)"
  else
    echo "GREEN  $label -- MUTATION SURVIVED, the claim is not pinned"
    fails=$((fails+1))
  fi
}

# $1 label, $2 file, $3 old, $4 new, $5 cargo test target
mutate_rs() {
  local label="$1" file="$2" old="$3" new="$4" target="$5"
  restore
  if ! apply "$file" "$old" "$new"; then
    echo "INERT  $label -- search string absent from $file; mutation never applied"
    fails=$((fails+1)); return
  fi
  # COMPILE FIRST, and separately. A mutation that does not build is INERT, not evidence: its red is
  # a build failure rather than a test disagreeing with it. It has to be its own step, because
  # `cargo test` prints its own `error: test failed, to rerun pass ...` on an ordinary red - so a
  # non-compile detector run over the test output reports every genuine PIN as inert, which is this
  # harness's own could-not-check-rendered-as-its-neighbour bug. It happened on the first run.
  if ! cargo check -p indexer-api --tests >/dev/null 2>&1; then
    echo "INERT  $label -- mutation does not compile; its red would be a build failure"
    fails=$((fails+1)); return
  fi
  out=$(cargo test -p indexer-api $target 2>&1)
  if echo "$out" | grep -qE "test result: FAILED"; then
    echo "PINNED $label -- $(echo "$out" | grep -oE "[0-9]+ passed; [0-9]+ failed" | head -1)"
  else
    echo "GREEN  $label -- MUTATION SURVIVED, the claim is not pinned"
    fails=$((fails+1))
  fi
}

echo "=== S-17 mutation evidence ==="

# ---- the client recomputation: THE security boundary ------------------------------------------
mutate "the recomputation skipped (the mirror is trusted instead)" \
  packages/ui/src/mirror/contentAddress.ts \
  "  return computed === requested" \
  "  return true || computed === requested" \
  test/contentAddress.test.ts

mutate "the hash algorithm ASSUMED to be keccak rather than read" \
  packages/ui/src/mirror/contentAddress.ts \
  "  if (hashAlgorithm !== MULTIHASH_KECCAK_256) {" \
  "  if (false) {" \
  test/contentAddress.test.ts

# ---- the chain of custody ----------------------------------------------------------------------
mutate "an unverified blob's contacts returned as published facts" \
  packages/ui/src/mirror/resolveProfile.ts \
  "  const parsed = parseProfileBlob(text);
  if (!parsed.ok) {" \
  "  const parsed = parseProfileBlob(text);
  if (false) {" \
  test/contentAddress.test.ts

mutate "a malformed logo entry degraded to 'no logo' instead of refusing the document" \
  packages/ui/src/mirror/profileBlob.ts \
  "    return { ok: false, reason: \"the profile document's logo names no valid content address\" };" \
  "    return { ok: true, profile: { contact, logo: null } };" \
  test/contentAddress.test.ts

# The TypeScript half of the hand-mirrored allowlist, and the twin of the Rust widening below. This
# is the list parseProfileBlob actually consults, and `application/json` is the specific value a
# reader "reconciling" it against mirror.rs::is_servable_media_type would add - which would let a
# LOGO entry name a JSON document that ProviderLogo then hands to an <img>.
mutate "the servable image set widened past its hand-mirrored Rust twin" \
  packages/ui/src/mirror/profileBlob.ts \
  '  "image/gif",
] as const;' \
  '  "image/gif",
  "application/json",
] as const;' \
  test/contentAddress.test.ts

mutate "the served media type believed rather than checked against what was published" \
  packages/ui/src/mirror/resolveProfile.ts \
  "  if (servedType !== logo.mediaType) {" \
  "  if (false) {" \
  test/contentAddress.test.ts

# ---- THE RENDERING RULE ------------------------------------------------------------------------
mutate "an unverified logo renders a placeholder image instead of nothing" \
  packages/ui/src/mirror/ProviderLogo.tsx \
  '  if (verified && url) {' \
  '  if (logo?.state === "unverified") {
    return <img src="/placeholder.png" alt={`${providerName} logo`} data-testid="provider-logo" />;
  }
  if (verified && url) {' \
  test/providerLogoRender.test.tsx

mutate "the object URL minted before verification lands (the pre-verification mount)" \
  packages/ui/src/mirror/ProviderLogo.tsx \
  '  const verified = logo?.state === "verified" ? logo : null;' \
  '  const verified = (logo as { bytes?: Uint8Array; mediaType?: string } | undefined)?.bytes
    ? (logo as unknown as { bytes: Uint8Array; mediaType: string })
    : logo === undefined
      ? ({ bytes: new Uint8Array([1]), mediaType: "image/png" } as const)
      : null;' \
  test/providerLogoRender.test.tsx

mutate "notPublished given the failure tone, so the two no-image states stop being distinguishable" \
  packages/ui/src/mirror/ProviderLogo.tsx \
  'className={cn("text-xs text-muted-foreground", className)}' \
  'className={cn("text-xs text-amber-700", className)}' \
  test/providerLogoRender.test.tsx

# ---- the publication ORDER ---------------------------------------------------------------------
mutate "the profile document no longer mirrored, so the anchor names content nothing holds" \
  packages/ui/src/provider/directoryPlan.ts \
  '  steps.push({
    kind: "mirrorUpload",
    what: "profile",' \
  '  if (false) steps.push({
    kind: "mirrorUpload",
    what: "profile",' \
  test/providerDirectoryPlan.test.ts

# ---- the listing preview: cannot-start must not wear the pending spelling ----------------------
mutate "cannot-start collapsed back into pending (the spinner that never resolves)" \
  packages/ui/src/domain/ProviderSelfServicePanel.tsx \
  "      {unconfigured ? (" \
  "      {false ? (" \
  test/publishedListingRender.test.tsx

mutate "an unverified document's contacts rendered as published facts" \
  packages/ui/src/domain/ProviderSelfServicePanel.tsx \
  '      ) : resolution.state === "unverified" ? (' \
  '      ) : false ? (' \
  test/publishedListingRender.test.tsx

# ---- the capacity caps: the store FILLS and refuses, and never evicts --------------------------
mutate_rs "the object cap removed (a stranger's token grows the oversight indexer's heap without bound)" \
  stacks/indexer/api/src/mirror.rs \
  "    if objects_after > MAX_MIRROR_OBJECTS {" \
  "    if false {" \
  "--lib"

mutate_rs "the total-byte cap removed (a few large objects exhaust the heap the object cap does not bound)" \
  stacks/indexer/api/src/mirror.rs \
  "    if bytes_after > MAX_MIRROR_TOTAL_BYTES {" \
  "    if false {" \
  "--lib"

mutate_rs "a re-publish counted as growth (the repair path refused at exactly the cap)" \
  stacks/indexer/api/src/mirror.rs \
  "    let objects_after = held_objects + usize::from(replacing.is_none());" \
  "    let objects_after = held_objects + 1;" \
  "--lib"

# ---- the uploads are UNCONDITIONAL, so lost content can be restored ----------------------------
mutate "the uploads gated on the anchor again (content lost from the ephemeral mirror is unrecoverable)" \
  packages/ui/src/provider/directoryPlan.ts \
  "  if (logo && logoAddress) {
    steps.push({
      kind: \"mirrorUpload\"," \
  "  if (!anchorUnchanged && logo && logoAddress) {
    steps.push({
      kind: \"mirrorUpload\"," \
  test/providerDirectoryPlan.test.ts

mutate "the profile upload gated on the anchor again (the reported no-repair sequence)" \
  packages/ui/src/provider/directoryPlan.ts \
  "  steps.push({
    kind: \"mirrorUpload\",
    what: \"profile\"," \
  "  if (!anchorUnchanged) steps.push({
    kind: \"mirrorUpload\",
    what: \"profile\"," \
  test/providerDirectoryPlan.test.ts

# ---- the mirror settings are refused BEFORE the loop, not inside it ----------------------------
mutate "the missing-token refusal dropped (publication aborts mid-sequence on 'missing bearer token')" \
  packages/ui/src/provider/directoryPlan.ts \
  "  if (!mirrorToken) {" \
  "  if (false) {" \
  test/providerDirectoryPlan.test.ts

mutate "the pre-flight answered only for a CHECKED plan (a config fault stays hidden until Publish)" \
  packages/ui/src/provider/directoryPlan.ts \
  "  if (steps && !steps.some((step) => step.kind === \"mirrorUpload\")) return null;" \
  "  if (!steps || !steps.some((step) => step.kind === \"mirrorUpload\")) return null;" \
  test/providerDirectoryPlan.test.ts

# ---- the logo picker validates, with a reason, rather than dropping ----------------------------
mutate "an oversized logo admitted by the picker (discovered as a 413 mid-publication instead)" \
  packages/ui/src/mirror/profileBlob.ts \
  "  if (bytes.length > MAX_CONTENT_BYTES) {" \
  "  if (false) {" \
  test/contentAddress.test.ts

mutate "an SVG logo admitted by the picker (a document that can carry script, in an operator portal)" \
  packages/ui/src/mirror/profileBlob.ts \
  "  if (!isServableImageMediaType(mediaType)) {
    return {
      ok: false," \
  "  if (false) {
    return {
      ok: false," \
  test/contentAddress.test.ts

# ---- the mirror itself (Rust) ------------------------------------------------------------------
mutate_rs "the mirror accepts content at an address it does not hash to" \
  stacks/indexer/api/src/mirror.rs \
  "    let computed = content_address(bytes);
    if computed != address {" \
  "    let computed = content_address(bytes);
    if false {" \
  "--test content_mirror"

mutate_rs "a corrupted row served instead of refused on the way out" \
  stacks/indexer/api/src/mirror.rs \
  "        if content_address(bytes) != address {
            return Ok(ContentRead::Corrupt);" \
  "        if false {
            return Ok(ContentRead::Corrupt);" \
  "--test content_mirror"

mutate_rs "SVG admitted to the mirror (it can carry script, and this renders in a portal)" \
  stacks/indexer/api/src/mirror.rs \
  '    &["image/png", "image/jpeg", "image/webp", "image/gif"];' \
  '    &["image/png", "image/jpeg", "image/webp", "image/gif", "image/svg+xml"];' \
  "--lib"

# The QUIET drift direction, and the reason the set is pinned as closed rather than by naming the one
# type that is obviously unsafe. An ordinary new raster format reddens the SVG test not at all, while
# the TypeScript mirror in packages/ui/src/mirror/profileBlob.ts would then refuse a logo the mirror
# had accepted - which fails the whole profile document and withholds that provider's contacts too.
mutate_rs "the servable image set widened past its hand-mirrored TypeScript twin" \
  stacks/indexer/api/src/mirror.rs \
  '    &["image/png", "image/jpeg", "image/webp", "image/gif"];' \
  '    &["image/png", "image/jpeg", "image/webp", "image/gif", "image/avif"];' \
  "--lib"

mutate_rs "the declared media type believed rather than sniffed (the allowlist bypass)" \
  stacks/indexer/api/src/mirror.rs \
  "    match sniff_media_type(bytes) {" \
  "    match Some(declared_media_type) {" \
  "--lib"

restore
if [ "$fails" -eq 0 ]; then
  echo "=== 0 mutation(s) unaccounted for ==="
else
  echo "=== $fails mutation(s) unaccounted for (GREEN or INERT above) ==="
fi
exit "$fails"

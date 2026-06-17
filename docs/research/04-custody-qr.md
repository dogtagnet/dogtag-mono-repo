# DogTag — Custody, Key-Genesis & QR/JWT Research (2025/2026)

Research target: a self-hosted Rust backend (vets/groomers business backend) that custodies
its own EVM keys, signs transactions, and shares records via a QR/JWT scheme.

Status as of **June 2026**. All crate recommendations verified against current docs.rs / GitHub.

---

## TL;DR recommended stack

| Concern | Recommendation |
| --- | --- |
| EVM signing / RPC | **`alloy`** (umbrella) + **`alloy-signer-local`** with features `mnemonic` + `keystore`. `ethers-rs` is **deprecated** — do not start new code on it. |
| Mnemonic (BIP39) | `coins-bip39` (re-exported by alloy as `alloy::signers::local::coins_bip39`). Direct: `bip39` crate. |
| HD derivation (BIP32/44) | `coins-bip32` (used internally by alloy `MnemonicBuilder`). Pure-alternative: `bip32` crate. |
| Keystore at rest | **eth keystore v3** (scrypt + AES-128-CTR + Keccak MAC) via alloy `keystore` feature, OR **`age`** for a simpler authenticated whole-seed blob. Recommendation below. |
| Secret-in-memory | `zeroize` + `secrecy`. |
| JWT | **`jsonwebtoken`** (Keats), alg **EdDSA (Ed25519)**, fallback **ES256**. |
| QR | `qrcode` crate (render) — payload = HTTPS deep link, ECC level **M**, byte mode. |
| keccak256 | **`tiny-keccak`** with `keccak` feature (Ethereum Keccak-256), or `sha3::Keccak256`. **Not** NIST SHA3-256. |

---

## 1. HD wallet key generation in Rust

### Crate landscape (current)

- **`ethers-rs` is deprecated.** The maintainer archived it in favour of **Alloy** + Foundry
  (gakonst/ethers-rs#2667). New code must target Alloy.
- **Alloy** is the maintained successor. For local custody you want:
  - `alloy` (umbrella) — providers, RPC, transaction types, primitives.
  - `alloy-signer-local` — `PrivateKeySigner`, `MnemonicBuilder`, keystore loading.
    Feature flags: `mnemonic` (BIP-39), `keystore` (eth keystore JSON).
  - Under the hood alloy uses `coins-bip39` and `coins-bip32` (the same crates that powered
    ethers). It re-exports the wordlist as `alloy::signers::local::coins_bip39::English`.
- Low-level alternatives (if you want to do derivation yourself, e.g. before alloy is even
  involved): `bip39` + `bip32` (RustCrypto-adjacent, pure Rust) or `coins-bip39`/`coins-bip32`.

### BIP39 — 24 words = 256-bit entropy

BIP39: 24 words encode **256 bits of entropy** (+ 8 checksum bits = 264 bits / 11 = 24 words).
Use the OS CSPRNG (`getrandom` / `OsRng`) for entropy — never a userspace PRNG.

```toml
# Cargo.toml
[dependencies]
alloy = { version = "1", features = ["full", "signer-local"] }
alloy-signer-local = { version = "1", features = ["mnemonic", "keystore"] }
coins-bip39 = "0.12"           # for explicit mnemonic generation/validation
rand = "0.8"
zeroize = "1"
secrecy = "0.10"
eyre = "0.6"
tokio = { version = "1", features = ["full"] }
```

#### Generate a 24-word mnemonic (256-bit entropy)

```rust
use coins_bip39::{English, Mnemonic};
use rand::thread_rng; // backed by OS CSPRNG seeding

// 24 words = 256 bits entropy
let mnemonic: Mnemonic<English> = Mnemonic::new_with_count(&mut thread_rng(), 24)?;
let phrase: String = mnemonic.to_phrase(); // SHOW ONCE to operator, then zeroize
```

Alloy can also generate during build via `MnemonicBuilder::build_random()` (shown below), but
generating the `Mnemonic` explicitly lets you control display/backup/zeroize before deriving.

#### Derive account N — path `m/44'/60'/0'/0/{index}`

`MnemonicBuilder` defaults to the Ethereum prefix `m/44'/60'/0'/0/` and `.index(n)` selects the
final component. (Verified from `alloy-rs/examples/wallets/mnemonic_signer.rs`.)

```rust
use alloy::signers::local::{coins_bip39::English, MnemonicBuilder, PrivateKeySigner};

fn account(phrase: &str, index: u32) -> eyre::Result<PrivateKeySigner> {
    let signer = MnemonicBuilder::<English>::default()
        .phrase(phrase)
        .index(index)?            // -> m/44'/60'/0'/0/{index}
        // .password("…")         // optional BIP39 passphrase ("25th word")
        .build()?;
    Ok(signer)
}

// account 0, account 1, … all from one seed:
let acct0 = account(phrase, 0)?;
let acct1 = account(phrase, 1)?;
println!("acct0 = {}", acct0.address());
```

Explicit-path form (e.g. a non-default change path) and random generation:

```rust
let w = MnemonicBuilder::<English>::default()
    .word_count(24)
    .derivation_path("m/44'/60'/0'/0/0")?
    .build_random()?;
```

#### Sign an EIP-1559 transaction + broadcast via JSON-RPC

`ProviderBuilder::new().wallet(signer).connect(rpc_url)` gives a wallet-enabled provider that
fills nonce/gas/chain-id and signs locally before `send_transaction`. (Tx shape verified from
`alloy-rs/examples/transactions/send_eip1559_transaction.rs`.)

```rust
use alloy::{
    network::TransactionBuilder,
    primitives::{U256, address},
    providers::{Provider, ProviderBuilder},
    rpc::types::TransactionRequest,
};

async fn send_1559(signer: alloy::signers::local::PrivateKeySigner) -> eyre::Result<()> {
    // wallet-filled provider: signs locally, broadcasts over JSON-RPC
    let provider = ProviderBuilder::new()
        .wallet(signer)
        .connect("https://rpc.your-chain.example") // or .connect_http(url)
        .await?;

    let bob = address!("d8dA6BF26964aF9D7eEd9e03E53415D37aA96045");
    let tx = TransactionRequest::default()
        .with_to(bob)
        .with_chain_id(provider.get_chain_id().await?)
        .with_value(U256::from(100))
        .with_gas_limit(21_000)
        .with_max_priority_fee_per_gas(1_000_000_000)   // EIP-1559: tip
        .with_max_fee_per_gas(20_000_000_000);          // EIP-1559: cap
        // (omit gas_price — that's the legacy field)

    let pending = provider.send_transaction(tx).await?;   // eth_sendRawTransaction under the hood
    let receipt = pending.get_receipt().await?;
    println!("mined in block {}", receipt.block_number.unwrap());
    Ok(())
}
```

**Legacy transaction:** drop the two 1559 fields and set `.with_gas_price(gwei)`. Alloy infers
the transaction type from which fields are populated (gas_price ⇒ legacy / type-0;
max_fee_per_gas ⇒ EIP-1559 / type-2).

#### Sign a message (EIP-191 / `personal_sign`)

`signer.sign_message(bytes)` applies the EIP-191 `"\x19Ethereum Signed Message:\n" + len` prefix
(this is exactly `personal_sign`). (Verified from `alloy-rs/examples/wallets/sign_message.rs`.)

```rust
use alloy::signers::Signer;

let sig = signer.sign_message(b"hello").await?;             // EIP-191 prefixed
let recovered = sig.recover_address_from_msg(b"hello")?;    // == signer.address()
```

For raw (non-prefixed) hashes use `sign_hash`. For typed data use `sign_dynamic_typed_data`
(EIP-712).

---

## 2. Key-at-rest security (self-hosted backend)

### The choice: eth keystore v3 vs age/libsodium

Two viable approaches; pick based on whether you want ecosystem compatibility.

**Option A — Web3 Secret Storage / eth keystore v3 (scrypt + AES-128-CTR + Keccak MAC).**
- This is the standard `UTC--…` keystore JSON used by geth/MetaMask/Foundry.
- KDF: scrypt (memory-hard, ASIC-resistant). Cipher: AES-128-CTR. Integrity: MAC =
  `keccak256(derivedKey[16..32] ++ ciphertext)` — checked *before* decryption to detect a wrong
  passphrase / tampering.
- **Caveat:** geth's *default* scrypt params are weak (`n=8192/N=2^13, r=8, p=1`). For an
  at-rest operator secret, bump to **`n=2^18` (262144), r=8, p=1**, which is the geth
  "StandardScryptN" hardened level. Alloy's `keystore` feature reads/writes this format.
- Pro: one encrypted file per account, drop-in compatible with the EVM tooling ecosystem.
- Con: keystore v3 encrypts a **single private key**, not the *seed*. For an HD wallet you want
  to encrypt the **mnemonic/seed** so you can derive more accounts later (see Option B).

**Option B (recommended for DogTag) — encrypt the *seed/mnemonic* with `age`.**
- DogTag is HD: the asset to protect is the **24-word mnemonic** (so new accounts can be derived
  on demand). Encrypt that single blob.
- **`age`** (`age` / `rage` crate, scrypt recipient mode) gives an authenticated, modern,
  audited file format: ChaCha20-Poly1305 AEAD payload, scrypt-derived key from the operator
  passphrase, built-in integrity (no hand-rolled MAC). Simpler and harder to misuse than rolling
  AES-CTR yourself.
- Alternative primitive: `libsodium` (`secretstream` / `crypto_secretbox`) + Argon2id KDF
  (`argon2` crate). Argon2id is the current OWASP-preferred password KDF; this is the most modern
  choice but you assemble KDF+AEAD yourself.

**Recommendation:** Store the mnemonic as an **`age`-encrypted seed blob** (passphrase mode) as
the source of truth, and *optionally* also export an eth keystore v3 for account 0 if you need
ecosystem interop. This keeps "derive account N later" trivial while giving authenticated
encryption with a vetted format.

### Unlock at boot & keep seed in memory

- **Passphrase source (in order of preference):** (1) interactive TTY prompt on start
  (best — never on disk/env), (2) a secrets manager / file the OS protects, (3) env var
  (`DOGTAG_UNLOCK_PASSPHRASE`) — acceptable for containers but visible in `/proc`, treat as
  lowest tier.
- **Decrypt once at boot** → hold the seed (or the per-account `SigningKey`) in a
  `secrecy::SecretBox<[u8; 32]>` / `Zeroizing` buffer. Derive `PrivateKeySigner`s on demand and
  let them drop.
- Wrap all key material in **`zeroize::Zeroizing`** / `secrecy::SecretBox` so it's wiped on drop
  and never `Debug`-printed or logged.
- Memory hardening (best-effort): `mlock` the seed page (`region`/`memsec` crate) to keep it off
  swap; disable core dumps (`RLIMIT_CORE=0`); never serialise the seed into traces.

### What NOT to do

- Do **not** store the mnemonic/seed/private key in plaintext, in the DB, in env without
  encryption, or in logs.
- Do **not** use geth's *default* scrypt params for a long-lived operator secret — harden them.
- Do **not** invent your own AES-CTR-without-MAC scheme; CTR has no integrity → use an AEAD
  (age/libsodium) or the keystore's Keccak MAC.
- Do **not** keep the decrypted seed in a plain `String`/`Vec<u8>` that lingers — use
  zeroizing types.
- Do **not** reuse the JWT signing key (section 4) for blockchain signing — separate key
  domains.

---

## 3. First-boot key-genesis flow

A one-shot, irreversible genesis state machine. The seed is generated, shown **once**, the
operator must confirm the backup, then it's encrypted and persisted.

### State machine

```
UNINITIALIZED
   └─(POST /admin/genesis/start  {passphrase})──────────────► PENDING_BACKUP
        • generate 24-word mnemonic (256-bit, OsRng)
        • hold in memory only (Zeroizing), NOT yet persisted
        • return { words[24], genesis_token }   (one-time, over TLS/localhost only)

PENDING_BACKUP
   └─(POST /admin/genesis/confirm {genesis_token, checksum_words})─► confirm backup
        • operator re-types e.g. words #5, #12, #23 (random challenge) to prove backup
        • on success: derive account 0 (m/44'/60'/0'/0/0)
        • encrypt seed with operator passphrase (age) → persist keystore.age
        • discard plaintext seed display; keep seed in memory unlocked
        ─────────────────────────────────────────────────────► INITIALIZED

INITIALIZED  (normal operation; subsequent boots → LOCKED → unlock with passphrase)
```

Persisted state: `genesis_done: bool`, `keystore.age`, an `accounts` table
`{ index, address, label, created_at }` (public data only — no key material).

### API sketch

| Method / path | Purpose | Notes |
| --- | --- | --- |
| `GET  /admin/genesis/status` | returns `UNINITIALIZED \| PENDING_BACKUP \| INITIALIZED \| LOCKED` | drives the frontend wizard |
| `POST /admin/genesis/start` | generate mnemonic, return words + `genesis_token` | refuse if already initialized; localhost/TLS + admin auth only |
| `POST /admin/genesis/confirm` | verify backup challenge, persist encrypted keystore | idempotency via `genesis_token`; transitions to INITIALIZED |
| `POST /admin/unlock` | on a normal boot, decrypt keystore with passphrase | LOCKED → INITIALIZED (in-memory) |
| `POST /admin/accounts` | derive next account `{ label }` → `{ index, address }` | derive `m/44'/60'/0'/0/index`, persist public row, return address |
| `GET  /admin/accounts` | list derived accounts (addresses + labels) | never returns private keys |

Frontend "derive additional account": calls `POST /admin/accounts` while INITIALIZED. The
backend keeps the seed unlocked in memory, derives the next index, stores only the new public
address + label, and returns it. No re-entry of the mnemonic.

---

## 4. Record-scoped JWT design

A short-lived token the backend mints so a mobile app can fetch **one** record. Encoded into a
QR alongside the API base URL and record id.

### Algorithm

- Each deployment owns its keys and is **both issuer and verifier** of these record tokens. That
  technically permits **HS256** (symmetric, simplest). **But** prefer asymmetric so the signing
  secret never needs to leave the backend and you keep the door open to client-side verification
  / key rotation via `kid`.
- **Recommendation: EdDSA (Ed25519).** Fastest sign+verify, 64-byte sigs, constant-time/
  side-channel resistant, best 2025/2026 greenfield choice.
  **Fallback: ES256** if a consuming platform lacks Ed25519. Avoid RS256 (large keys, slower).
- Generate a **per-deployment** Ed25519 keypair at genesis (separate from the blockchain seed).
  Publish the public key (or a JWKS with `kid`) if any party other than the backend ever verifies.

### Claim set

```jsonc
{
  "iss": "https://clinic-42.dogtag.app",   // this deployment (must match QR base URL)
  "sub": "rec_01HXYZ…",                     // the recordId — the ONLY record this grants
  "aud": "dogtag-mobile",                   // intended client audience
  "scope": "read:record",                   // least privilege; read-only
  "iat": 1718600000,
  "nbf": 1718600000,
  "exp": 1718600300,                         // ~5 minutes
  "jti": "9f1c…"                             // unique → one-time-use store key
}
```

- `exp`: **minutes, not hours** — 2–5 min for a "scan now" flow. Short TTL is the primary defense.
- `sub = recordId` is the scoping mechanism; the route handler **must** assert
  `token.sub == path recordId` and reject otherwise (defense against token reuse on another id).
- `scope = read:record` enforces read-only.
- `kid` header if you rotate keys / run JWKS.

### Mint + verify (Rust, `jsonwebtoken` 10.x)

```toml
jsonwebtoken = "10"
```

```rust
use jsonwebtoken::{encode, decode, Header, EncodingKey, DecodingKey, Validation, Algorithm};

// MINT (EdDSA)
let mut header = Header::new(Algorithm::EdDSA);
header.kid = Some(deployment_kid.clone());
let token = encode(&header, &claims, &EncodingKey::from_ed_der(&ed25519_priv_der))?;

// VERIFY in GET /records/{id}
let mut v = Validation::new(Algorithm::EdDSA);
v.set_audience(&["dogtag-mobile"]);
v.set_issuer(&[my_deployment_url]);
v.set_required_spec_claims(&["exp", "sub", "aud", "iss"]);
v.leeway = 30;                       // clock-skew tolerance (default 60; 5–30s recommended)
v.validate_exp = true;
let data = decode::<Claims>(token, &DecodingKey::from_ed_der(&ed25519_pub_der), &v)?;

// scope + path binding (NOT done by the lib — you must enforce):
ensure!(data.claims.sub == path_record_id, "wrong record");
ensure!(data.claims.scope == "read:record", "bad scope");
```

`jsonwebtoken` auto-validates `exp`/`nbf` and (when configured) `aud`/`iss`; it does **not** know
about `scope`, `sub==recordId`, or `jti` — enforce those yourself.

### One-time-use (jti) enforcement

- Keep a **server-side jti store** with TTL = token `exp`. On first successful fetch, atomically
  `INSERT jti` (or `SETNX jti EX <ttl>` in Redis); if it already exists → **reject** as replay.
- This makes the token genuinely one-shot. If you instead want "scan many times within 5 min,"
  skip jti consumption and rely on `exp` alone — choose per UX.
- The store only needs entries until `exp`; expired jtis self-evict, so it stays small.

### Clock skew

- Allow **bounded leeway of ~30 s** (`Validation::leeway`); never more than ~60 s. Larger windows
  weaken the short-TTL guarantee. Keep deployment clocks NTP-synced.

---

## 5. QR payload format

### Constraints

- **Many deployments → the base URL varies**, so it must travel inside the payload (you can't bake
  a fixed host into the app).
- Keep it small & robust. QR byte-mode capacity is generous (v40 = 2953 bytes) but **smaller QR =
  easier scan**. A token + url + id at ECC **M** sits comfortably around **QR version 6–10**.
- Error correction **M (~15%)** is the business sweet spot (durable enough for a printed/displayed
  code without wasting capacity). Use **Q/H** only if printed small or on a glossy/abused surface.

### Recommended format: HTTPS deep link

A single absolute HTTPS URL — universally scannable, opens the app via universal/app links, and
embeds everything:

```
https://<deployment-host>/r?t=<jwt>&i=<recordId>
```

- `t` = the EdDSA JWT (already URL-safe base64url; no extra encoding).
- `i` = recordId.
- The **scheme+host** *is* the API base URL → no separate `url` field needed; the mobile app
  derives the API base from the link's origin. This elegantly solves "URL varies per deployment."
- App flow: scan → resolve origin as API base → `GET {origin}/records/{i}` with
  `Authorization: Bearer {t}` → backend verifies JWT (section 4) → returns record.

Why a URL over a raw JSON blob:
- A JSON blob (`{"url":…,"jwt":…,"recordId":…}`) forces byte mode, is bulkier, and a generic
  camera/QR app shows gibberish instead of a tappable link. The URL degrades gracefully — even a
  stock camera opens it; the universal link hands off to the DogTag app if installed.

**Size tip:** the JWT dominates payload size. EdDSA (64-byte sig) keeps it compact vs RS256.
Trim claims to the minimum (short `iss`, opaque short `sub`/`recordId`). Keep total URL well under
~300 chars → low QR version, fast scans. If you ever blow past capacity, switch ECC down to L or
issue an opaque short reference token and look up the JWT server-side.

### Rendering in Rust

```toml
qrcode = "0.14"      # generate the matrix / SVG / image
```

```rust
use qrcode::{QrCode, EcLevel};
let url = format!("https://{host}/r?t={jwt}&i={record_id}");
let code = QrCode::with_error_correction_level(url.as_bytes(), EcLevel::M)?;
// render to SVG or image::ImageBuffer for the dashboard
```

---

## 6. keccak256 in Rust (for the merkle work)

**Critical distinction:** Ethereum uses **Keccak-256** (the original SHA-3 contest submission),
**not** NIST **SHA3-256**. The two share the same permutation but differ in the **padding byte**
(`0x01` for original Keccak vs `0x06` for FIPS-202 SHA-3). They produce **different digests** —
using SHA3-256 will silently give wrong hashes/merkle roots/addresses.

### Crate choice

- **`tiny-keccak`** (feature `keccak`) — small, fast (fully unrolled f[1600]), the canonical
  Ethereum choice.
- **`sha3`** (RustCrypto) — provides `Keccak256` *and* `Sha3_256` as distinct types.
- alloy/alloy-primitives already expose **`alloy_primitives::keccak256(bytes) -> B256`**; if alloy
  is in the tree, just use that and skip a direct dependency.

```toml
tiny-keccak = { version = "2", features = ["keccak"] }
# or
sha3 = "0.10"
```

```rust
// tiny-keccak — Ethereum Keccak-256
use tiny_keccak::{Hasher, Keccak};
fn keccak256(data: &[u8]) -> [u8; 32] {
    let mut k = Keccak::v256();   // ORIGINAL Keccak (Ethereum), NOT sha3
    k.update(data);
    let mut out = [0u8; 32];
    k.finalize(&mut out);
    out
}

// RustCrypto sha3 — pick Keccak256 (correct), NOT Sha3_256
use sha3::{Digest, Keccak256};
let digest = Keccak256::digest(data);     // correct for Ethereum
// let wrong = sha3::Sha3_256::digest(data); // DIFFERENT padding — DO NOT use for EVM

// If alloy is present:
let h = alloy_primitives::keccak256(data); // B256
```

For the merkle tree, hash leaves and internal nodes with this same Keccak-256 and keep the
domain-separation/ordering convention consistent (e.g. sorted-pair or position-tagged) so it
matches whatever on-chain/off-chain verifier you target.

---

## Sources

- Alloy — site & migration: <https://alloy.rs/> · <https://alloy.rs/migrating-from-ethers/reference/>
- `alloy-signer-local` docs: <https://docs.rs/alloy-signer-local/latest/alloy_signer_local/>
- Alloy examples (verified code): <https://github.com/alloy-rs/examples>
  (`wallets/mnemonic_signer.rs`, `wallets/sign_message.rs`, `wallets/keystore_signer.rs`,
  `transactions/send_eip1559_transaction.rs`)
- ethers-rs deprecation: <https://github.com/gakonst/ethers-rs/issues/2667>
- `ethers_signers::MnemonicBuilder` (default ETH path reference): <https://docs.rs/ethers-signers/latest/ethers_signers/struct.MnemonicBuilder.html>
- `bip32` crate: <https://docs.rs/bip32>
- Web3 Secret Storage Definition (keystore v3, scrypt + AES-128-CTR + Keccak MAC): <https://ethereum.org/developers/docs/data-structures-and-encoding/web3-secret-storage/>
- Ethereum wallet encryption (scrypt params, MAC): <https://cryptobook.nakov.com/symmetric-key-ciphers/ethereum-wallet-encryption>
- `age` encryption: <https://github.com/FiloSottile/age> · `age` Rust crate: <https://docs.rs/age>
- `zeroize`: <https://docs.rs/zeroize> · `secrecy`: <https://docs.rs/secrecy>
- JWT algorithms (ES256/EdDSA/HS256): <https://www.scottbrady.io/jose/jwts-which-signing-algorithm-should-i-use> · <https://workos.com/blog/hmac-vs-rsa-vs-ecdsa-which-algorithm-should-you-use-to-sign-jwts>
- JWT best practices (exp, jti, leeway): <https://curity.io/resources/learn/jwt-best-practices/>
- `jsonwebtoken` crate & `Validation`: <https://github.com/Keats/jsonwebtoken> · <https://docs.rs/jsonwebtoken/latest/jsonwebtoken/struct.Validation.html>
- QR capacity/versions/ECC: <https://www.qrcode.com/en/about/version.html> · <https://www.qrcodechimp.com/qr-code-storage-capacity-guide/>
- `qrcode` crate: <https://docs.rs/qrcode>
- Keccak-256 vs SHA3-256 (padding difference; Ethereum uses Keccak): <https://github.com/status-im/nim-keccak-tiny/issues/1> · <https://www.cybertest.com/blog/keccak-vs-sha3>
- `tiny-keccak`: <https://github.com/debris/tiny-keccak> · <https://docs.rs/tiny-keccak> · `sha3`: <https://docs.rs/sha3>

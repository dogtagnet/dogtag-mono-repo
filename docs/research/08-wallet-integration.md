# 08 — Wallet Integration Research: DogTag on ROAX

> Target chain: **ROAX** — chainId `0x87` (`135` decimal), RPC `https://devrpc.roax.net`,
> native gas token **PLASMA**, Blockscout explorer `https://explorer.roax.net`.
> Date: 2026-06-17.
>
> Two integrations, distinct audiences:
> - **PART A** — VET/GROOMER web apps (React + Vite + TS): dual, mutually-exclusive,
>   switchable signing — MetaMask **OR** self-hosted Rust backend custody.
> - **PART B** — Consumer MOBILE app (Android Kotlin + iOS Swift): an in-app
>   self-custodial EVM wallet under Settings, "like Telegram does it."
>
> Builds on the existing system docs: `03-chain-contracts.md` (`IssuerRegistry`,
> `DogTagIssuer`, `DogTagSBT`, merkle-root anchoring, `isWhitelistedFor(recordType, signer)`),
> `02-attestation.md` (wrapped-document / merkle `targetHash`/`merkleRoot`/`proof`), and
> `04-custody-qr.md` (Alloy-based Rust backend, HD key genesis, QR/JWT). The shared SDK
> packages are `packages/dogtag-standard-ts` and `crates/dogtag-standard-rs`.

---

# PART A — Web apps: dual switchable signing (MetaMask vs backend custody)

## A0. The core architectural idea

A vet/groomer must be able to issue a credential (anchor a merkle root on a `DogTagIssuer`,
or mint a `DogTagSBT`) using **either**:

- **(a) Wallet mode** — their own browser wallet (MetaMask, or any EIP-1193/WalletConnect
  wallet via Reown), signing+broadcasting from their personal EVM address, paying PLASMA gas
  themselves; or
- **(b) Backend mode** — the self-hosted Rust backend (`crates/dogtag-standard-rs` consumer)
  holding an HD seed (per `04-custody-qr.md`), which signs+broadcasts from a backend-derived
  address and pays gas from a funded key.

The two modes must be **mutually exclusive, switchable at any time, and behaviourally
identical** in everything except *who signs and who pays gas*. The decisive design rule:

> **The wrapped-document/merkle-root building is IDENTICAL in both modes.** It lives in the
> shared SDK / backend — never duplicated in the wallet path. The only thing that differs is
> the final "sign + broadcast" step.

This is achieved with a `SigningStrategy` interface (A2) and a backend that always returns an
**unsigned transaction** `{ to, data, value, chainId }` for wallet mode while signing itself
in backend mode.

---

## A1. Browser-wallet stack (wagmi v2-era + viem + Reown AppKit)

### A1.1 Package versions (verified npm, 2026-06-17)

| Package | Version |
|---|---|
| `wagmi` | **3.6.17** (still the "v2-era" API: `createConfig`, `WagmiProvider`, renamed hooks) |
| `viem` | **2.52.2** |
| `@reown/appkit` | **1.8.21** |
| `@reown/appkit-adapter-wagmi` | **1.8.21** (version-lock to `@reown/appkit`) |
| `@tanstack/react-query` | **5.101.0** (mandatory peer — wagmi delegates async state to it) |

```bash
npm install wagmi viem @tanstack/react-query @reown/appkit @reown/appkit-adapter-wagmi
```

`wagmi` replaced ethers.js with **viem** and delegates caching to **TanStack Query**, so
`QueryClientProvider` is required alongside `WagmiProvider`. **Reown AppKit** is the rebrand of
WalletConnect's Web3Modal; it needs a free **`projectId`** from <https://dashboard.reown.com>
and bundles the WalletConnect v2 connector automatically.

### A1.2 Define the ROAX/PLASMA chain (viem `defineChain`)

```ts
// chains/roax.ts
import { defineChain } from 'viem'

export const roax = defineChain({
  id: 135,                                    // 0x87
  name: 'ROAX',
  nativeCurrency: { name: 'Plasma', symbol: 'PLASMA', decimals: 18 },
  rpcUrls: { default: { http: ['https://devrpc.roax.net'] } },
  blockExplorers: { default: { name: 'ROAX Explorer', url: 'https://explorer.roax.net' } },
})
```

### A1.3 Reown AppKit + wagmi provider wiring

```tsx
// AppKitProvider.tsx
import { createAppKit } from '@reown/appkit/react'
import { WagmiProvider } from 'wagmi'
import { QueryClient, QueryClientProvider } from '@tanstack/react-query'
import { WagmiAdapter } from '@reown/appkit-adapter-wagmi'
import { roax } from './chains/roax'

const queryClient = new QueryClient()
const projectId = import.meta.env.VITE_REOWN_PROJECT_ID   // dashboard.reown.com
const networks = [roax]

const wagmiAdapter = new WagmiAdapter({ networks, projectId, ssr: false })

createAppKit({
  adapters: [wagmiAdapter],
  networks,
  projectId,
  metadata: {
    name: 'DogTag Vet Portal',
    description: 'DogTag credential issuance',
    url: 'https://vet.dogtag.app',           // must match your domain
    icons: ['https://vet.dogtag.app/icon.png'],
  },
})

export function AppKitProvider({ children }: { children: React.ReactNode }) {
  return (
    <WagmiProvider config={wagmiAdapter.wagmiConfig}>
      <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
    </WagmiProvider>
  )
}
```

Connect via the prebuilt `<appkit-button />` web component or `useAppKit()`. All wagmi hooks
work inside this tree because `wagmiAdapter.wagmiConfig` is a real wagmi `Config`. (Plain
`createConfig` with `injected()`, `metaMask()`, `walletConnect({ projectId })` connectors is
the non-Reown alternative.)

### A1.4 Add/switch to ROAX — `wallet_addEthereumChain` (EIP-3085)

`wallet_switchEthereumChain` takes only `{ chainId }`; if the chain is unknown the wallet
errors with code **4902**, which is the cue to call `wallet_addEthereumChain`. `chainId` must
be a **0x-prefixed hex string** (`'0x87'`).

**Raw EIP-1193:**
```ts
async function ensureRoax() {
  const params = {
    chainId: '0x87',                                   // 135
    chainName: 'ROAX',
    nativeCurrency: { name: 'Plasma', symbol: 'PLASMA', decimals: 18 },
    rpcUrls: ['https://devrpc.roax.net'],
    blockExplorerUrls: ['https://explorer.roax.net'],
  }
  try {
    await window.ethereum.request({
      method: 'wallet_switchEthereumChain', params: [{ chainId: '0x87' }],
    })
  } catch (err: any) {
    if (err.code === 4902) {
      await window.ethereum.request({ method: 'wallet_addEthereumChain', params: [params] })
    } else throw err
  }
}
```

**wagmi way** — register `roax` in the config, then `useSwitchChain`. wagmi auto-issues
`wallet_switchEthereumChain` and, if the chain is configured but unknown to the wallet, issues
`wallet_addEthereumChain` for you (deriving params from the `defineChain` definition):

```tsx
import { useSwitchChain, useChainId } from 'wagmi'
import { roax } from './chains/roax'

function EnsureRoax() {
  const { switchChain, isPending } = useSwitchChain()
  const chainId = useChainId()
  if (chainId === roax.id) return null
  return (
    <button disabled={isPending} onClick={() => switchChain({ chainId: roax.id })}>
      Switch to ROAX
    </button>
  )
}
```

### A1.5 Build calldata, send tx, get txHash, await receipt

Wallet mode submits the **unsigned tx the backend returns** (A3) as a raw `{ to, data, value }`
via `useSendTransaction`, then waits with `useWaitForTransactionReceipt`:

```tsx
import { useSendTransaction, useWaitForTransactionReceipt } from 'wagmi'

function SubmitUnsigned({ tx }: { tx: { to: `0x${string}`; data: `0x${string}`; value?: bigint } }) {
  const { data: hash, sendTransaction, isPending, error } = useSendTransaction()
  const { isLoading: confirming, isSuccess: confirmed, data: receipt } =
    useWaitForTransactionReceipt({ hash })

  return (
    <>
      <button disabled={isPending}
        onClick={() => sendTransaction({ to: tx.to, data: tx.data, value: tx.value ?? 0n })}>
        {isPending ? 'Confirm in wallet…' : 'Issue credential'}
      </button>
      {hash && <p>txHash: {hash}</p>}
      {confirming && <p>Mining…</p>}
      {confirmed && <p>Confirmed in block {receipt.blockNumber.toString()}</p>}
      {error && <p>{error.message}</p>}
    </>
  )
}
```

If the frontend ever needs to build calldata itself (it generally should NOT — see A2), it uses
viem's `encodeFunctionData({ abi, functionName, args })`. For ordinary contract calls the
high-level `useWriteContract` encodes calldata for you. Naming note for wagmi v2-era:
`useContractWrite` → **`useWriteContract`**, `useWaitForTransaction` → **`useWaitForTransactionReceipt`**;
the write hook's `data` is now the hash directly (a `0x${string}`), not `{ hash }`.

---

## A2. The `SigningStrategy` abstraction (mutually-exclusive, switchable)

### A2.1 Interface

Both modes implement one interface. The frontend code that *builds* a credential never knows
which mode is active; it only calls `submit(...)` and gets back a `txHash` + the persisted
record id.

```ts
// packages/dogtag-standard-ts/src/signing/strategy.ts

/** What the backend always produces: a ready-to-send, unsigned EVM tx. */
export interface UnsignedTx {
  to: `0x${string}`
  data: `0x${string}`
  value: bigint          // PLASMA wei; usually 0n for issue/mint
  chainId: 135           // ROAX
}

/** The credential the backend has wrapped (merkle root etc.) — see A2.2. */
export interface PreparedCredential {
  recordId: string                 // server-side draft id (idempotency anchor)
  recordType: string               // VACCINATION | OWNERSHIP | LICENSE | ...
  merkleRoot: `0x${string}`        // the value to anchor on-chain
  targetHash: `0x${string}`
  proof: `0x${string}`[]           // empty for single-record (root === targetHash)
  wrappedDocument: unknown         // canonical wrapped doc (off-chain payload)
  unsignedTx: UnsignedTx           // {to,data,value,chainId} to anchor merkleRoot
}

export interface SubmitResult {
  recordId: string
  txHash: `0x${string}`
  signerAddress: `0x${string}`     // who actually signed (for whitelist audit)
  mode: 'wallet' | 'backend'
}

export interface SigningStrategy {
  readonly mode: 'wallet' | 'backend'
  /** The address that will sign — MUST be whitelisted on-chain (A3). */
  activeSignerAddress(): Promise<`0x${string}`>
  /** Sign + broadcast the prepared credential's tx; return txHash + persist. */
  submit(prepared: PreparedCredential): Promise<SubmitResult>
  /** Health/connection check for the settings UI. */
  status(): Promise<{ connected: boolean; detail?: string }>
}
```

### A2.2 Where the merkle/wrapped-document build happens — ALWAYS the backend/SDK

The wrapped-document and merkle-root logic from `02-attestation.md` (`wrapDocument` →
`{ targetHash, proof, merkleRoot }`, single-record convention `merkleRoot === targetHash` with
empty proof, sorted-pair Keccak-256 hashing, leaf bound to `tokenId` +`recordType` +
`payloadHash`) is **identical regardless of signing mode**. To guarantee that, it is computed
**once, server-side**, by the Rust backend using `crates/dogtag-standard-rs` (or, if a pure-JS
path is ever needed, the mirror logic in `packages/dogtag-standard-ts` — the two MUST produce
byte-identical roots; this is a cross-language conformance-test requirement).

The frontend NEVER hand-builds calldata for issuance. Instead it calls:

```
POST /api/credentials/prepare   { recordType, petTokenId, payload, mode }
  -> 200 PreparedCredential       (recordId draft + merkleRoot + unsignedTx{to,data,value,chainId})
```

- For **both modes** the backend does the wrap + merkle + `encodeFunctionData`-equivalent
  (Alloy `IDogTagIssuer::issue(merkleRoot)` calldata) and returns `unsignedTx`.
- In **backend mode** the same endpoint (or a `mode:"backend"` flag) tells the backend to go
  ahead and sign+broadcast itself (Alloy `provider.send_transaction`, per `04-custody-qr.md`),
  returning the `txHash` directly — no `unsignedTx` round-trip to the wallet.

This keeps "what gets anchored" provably mode-independent.

### A2.3 The two implementations

```ts
// WalletStrategy — browser wallet signs the backend's unsigned tx
import { sendTransaction, waitForTransactionReceipt, getAccount } from '@wagmi/core'
import type { Config } from 'wagmi'

export class WalletStrategy implements SigningStrategy {
  readonly mode = 'wallet' as const
  constructor(private cfg: Config, private api: DogTagApi) {}

  async activeSignerAddress() {
    const acct = getAccount(this.cfg)
    if (!acct.address) throw new Error('wallet not connected')
    return acct.address
  }

  async submit(p: PreparedCredential): Promise<SubmitResult> {
    const signer = await this.activeSignerAddress()
    // (Optional pre-flight) confirm signer is whitelisted for this recordType — A3
    const hash = await sendTransaction(this.cfg, {
      to: p.unsignedTx.to, data: p.unsignedTx.data,
      value: p.unsignedTx.value, chainId: 135,
    })
    await waitForTransactionReceipt(this.cfg, { hash })
    // Persist consistently via the SAME backend endpoint backend-mode uses (A2.4)
    await this.api.confirmCredential({ recordId: p.recordId, txHash: hash, signer })
    return { recordId: p.recordId, txHash: hash, signerAddress: signer, mode: 'wallet' }
  }

  async status() {
    const a = getAccount(this.cfg)
    return { connected: !!a.address, detail: a.address }
  }
}
```

```ts
// BackendStrategy — backend signs + broadcasts with its HD-derived key
export class BackendStrategy implements SigningStrategy {
  readonly mode = 'backend' as const
  constructor(private api: DogTagApi) {}

  async activeSignerAddress() {
    return (await this.api.backendSignerAddress()).address   // GET /admin/accounts active
  }

  async submit(p: PreparedCredential): Promise<SubmitResult> {
    // Backend already has (or re-derives) the prepared credential; ask it to sign+broadcast.
    const r = await this.api.issueWithBackend({ recordId: p.recordId })  // returns {txHash, signer}
    return { recordId: p.recordId, txHash: r.txHash, signerAddress: r.signer, mode: 'backend' }
  }

  async status() {
    const s = await this.api.backendStatus()   // INITIALIZED/LOCKED + funded balance
    return { connected: s.state === 'INITIALIZED', detail: `${s.signer} (${s.plasmaBalance} PLASMA)` }
  }
}
```

A small factory/React context resolves the active strategy from a per-issuer setting:

```ts
function makeStrategy(mode: 'wallet' | 'backend', cfg: Config, api: DogTagApi): SigningStrategy {
  return mode === 'wallet' ? new WalletStrategy(cfg, api) : new BackendStrategy(api)
}
```

### A2.4 Consistent persistence in BOTH modes

The record is persisted by the **backend, through one code path**, regardless of who signed:

```
POST /api/credentials/confirm  { recordId, txHash, signer }     // wallet mode (frontend reports)
   — backend verifies on-chain: receipt success + the issuer's RootIssued(merkleRoot, signer)
     event matches the draft's merkleRoot, then flips draft -> issued.

(backend mode) the issue+persist happen atomically inside the backend after broadcast.
```

Persisted row (identical schema both modes): `{ recordId, recordType, petTokenId, merkleRoot,
targetHash, proof, wrappedDocumentRef, txHash, signerAddress, mode, issuedAt, status }`. The
`mode`/`signerAddress` columns are audit metadata only — verification and downstream behaviour
ignore them. Crucially, the backend should **re-derive/verify** the on-chain state (read the
`DogTagIssuer.issuedAt[merkleRoot]` and the `RootIssued` event) before marking `issued`, so a
lying or buggy frontend can't mark an unissued record as issued.

---

## A3. Whitelist implication — multiple whitelisted addresses per issuer

### A3.1 The problem

Per `03-chain-contracts.md`, `DogTagIssuer.issue()`/`revoke()` are gated by
`registry.isWhitelistedFor(recordType, msg.sender)` (or `isWhitelisted(msg.sender)` in the
simpler variant). The **signer's address** must hold `ISSUER_ROLE`. But a vet may sign with
**either** their MetaMask EOA **or** their backend-derived address — these are *different
addresses*. Therefore **both addresses must be whitelistable for the same logical issuer**.

### A3.2 Design — issuer identity ⊋ signing addresses (one-to-many)

The registry already supports this naturally because `AccessControl` grants a role to an
**address**, and an issuer can have **many** addresses granted. Model an *issuer* (the vet/clinic
business entity) as a set of signing addresses, all whitelisted for the record types they may
issue:

```solidity
// Conceptual extension of IssuerRegistry (03-chain-contracts.md §3.3 per-recordType variant)
// recordType => signer => bool
mapping(bytes32 => mapping(address => bool)) private _whitelist;

function isWhitelistedFor(bytes32 recordType, address signer) external view returns (bool) {
    return _whitelist[recordType][signer];
}

// admin-gated
function whitelistFor(bytes32 recordType, address signer) external onlyRole(DEFAULT_ADMIN_ROLE) {
    _whitelist[recordType][signer] = true;
    emit SignerWhitelisted(recordType, signer);          // index off-chain -> issuer entity
}
function delistFor(bytes32 recordType, address signer) external onlyRole(DEFAULT_ADMIN_ROLE) {
    _whitelist[recordType][signer] = false;
    emit SignerDelisted(recordType, signer);
}
```

Off-chain, an `issuer_entity` row links the business to its addresses:
`issuer_entity { id, name } 1—* issuer_signer { issuerEntityId, address, mode('wallet'|'backend'),
recordTypes[], whitelistedTxHash, status }`. The protocol admin whitelists each address on-chain;
the off-chain table is the human-friendly view (the contract has no concept of "the same vet").

### A3.3 Invariant + onboarding flow

**Invariant:** *the active signing address must be whitelisted for the record type being issued.*
The `SigningStrategy.submit()` should pre-flight `isWhitelistedFor(recordType, activeSigner)` via
RPC (`eth_call`) and fail fast with a clear "this address isn't approved yet" message rather than
letting the tx revert and burn gas.

**Onboarding when switching modes:** switching from backend→wallet (or registering a second
device wallet) introduces a *new* signer address that is very likely **not yet whitelisted**.
Flow:

1. User connects the new wallet (or backend genesis derives the backend address).
2. App reads `isWhitelistedFor(recordType, newAddress)` for each record type → shows status.
3. If not whitelisted, the app submits a **whitelist request** to the protocol admin
   (off-chain approval queue) carrying `{ issuerEntityId, address, mode, recordTypes }`.
4. Protocol admin calls `whitelistFor(recordType, address)` (admin-gated, admin pays gas).
5. App polls `SignerWhitelisted` / `isWhitelistedFor` until true, then enables issuing in that mode.

### A3.4 Pitfalls

- **Silent revert / wasted gas:** issuing from a non-whitelisted address reverts `onlyWhitelisted`.
  Always pre-flight via `eth_call` (A3.3) — especially in wallet mode where the *user* pays.
- **Per-recordType scoping mismatch:** an address whitelisted for `VACCINATION` is **not**
  automatically allowed for `OWNERSHIP`. Whitelist every record type the issuer needs, per
  address. The UI must show a per-(address × recordType) matrix.
- **Stale/over-broad whitelisting:** whitelisting "both" addresses permanently widens the attack
  surface — if the MetaMask key leaks, it can issue even while the clinic only "uses" backend
  mode. Mitigation: **delist the inactive mode's address** on switch (optional, stricter), or at
  least support fast `delistFor` revocation (registry revocation is O(1) and disables the signer
  across all issuers, per `03-chain-contracts.md`).
- **Backend key rotation:** if the backend re-derives a new account index (`m/44'/60'/0'/0/n`),
  that is a *new address* requiring whitelisting. Treat backend account rotation as an onboarding
  event, not a silent change.
- **Admin centralisation:** only the protocol admin can whitelist, so onboarding is gated on
  admin turnaround. Build the approval queue + notifications so this isn't a hidden bottleneck.
- **Address confusion in audit:** persist `signerAddress` + `mode` on every record (A2.4) so an
  auditor can see which key signed which credential — essential after a key compromise.

---

## A4. UX — settings toggle, status, in-flight, gas

- **Settings toggle:** a single per-issuer "Signing method" control: *Browser wallet* ⟷
  *Server-managed key*. Mutually exclusive radio, persisted server-side (so it follows the user
  across devices) and reflected by the active `SigningStrategy`.
- **Connection status panel:**
  - Wallet mode → wagmi `useAccount()` (connected address, ENS), `useChainId()` (must be ROAX
    135 — show "Switch to ROAX" via A1.4 if not), and a whitelist badge per record type (A3.3).
  - Backend mode → `GET /admin/genesis/status` (`INITIALIZED`/`LOCKED`), the active backend
    address, and its **PLASMA balance** (gas funding health).
- **In-flight records when switching:** a switch changes only *future* signing. Records already
  **broadcast** (have a `txHash`) are unaffected — they confirm/persist under whichever address
  signed them. Records in **draft/prepared** state (have a `merkleRoot` but no `txHash`) should
  be **re-validated** against the new active signer: re-run the A3.3 whitelist pre-flight, and if
  the prepared `unsignedTx` was mode-specific, re-`prepare` (the merkleRoot is mode-independent,
  so the draft/recordId is reusable; only the broadcast path changes). Never let a switch
  silently submit a half-built record under the wrong key. Block switching while a submit is
  pending (`isPending`) to avoid ambiguous attribution.
- **Gas / PLASMA funding differences:**
  - **Wallet mode** → the *user's own* address pays PLASMA gas. They must hold PLASMA. Surface a
    balance warning and a faucet/funding link; handle "insufficient funds" cleanly.
  - **Backend mode** → the *backend's funded key* pays. Operator must keep it topped up; the
    status panel should warn below a threshold. Users issue "gaslessly" from their perspective.
  - This is the single biggest day-to-day UX difference and should be stated plainly in the
    toggle's helper text ("Browser wallet: you pay PLASMA gas. Server key: the clinic's wallet
    pays.").

---

# PART B — Mobile self-custodial wallet ("like Telegram")

## B1. How Telegram does it (UX/security model to borrow from)

Naming churn first: the in-app **"Wallet"** (Settings → Wallet, or @wallet) contains two modes —
a **custodial Crypto Wallet** (operated by a third party, The Open Platform/Wallet; keys held for
you, auth via Telegram + 2FA) and a **self-custodial** wallet launched 2024 as **TON Space**,
later renamed **TON Wallet**, and ~May 2026 **"DeFi Account."** All three names = the same
self-custodial wallet.

**Self-custodial (TON Space) key facts:**
- Generates a secret key from a randomly generated **24-word BIP-39 seed phrase**, stored **only
  on device**; "Neither Crypto Wallet nor Telegram have access to it." Optional in-app passcode +
  **biometrics** (Face ID / fingerprint).
- **Two backup paths:**
  1. **Manual** — user writes the 24 words; classic self-custody, classic single point of failure.
  2. **Email/cloud "seedless-style" backup** — a decryption key is generated on device and used to
     encrypt the seed; the encrypted seed is **split into shards** distributed across three parties
     (Crypto Wallet holds one shard + the decryption key; Telegram holds another shard *without*
     the decryption key; the wallet service keeps its shard separate) so **no single party can
     reconstruct** the seed. Recovery uses an email code **and only works while logged into the
     Telegram account** (account control = second factor).
- **dApps:** via **TON Connect** — one-tap connect from Telegram Mini Apps/games/DeFi.

**Critical nuance:** Telegram's "seedless"/"split-key" is **encrypted-seed Shamir-style sharding
for backup, NOT true MPC threshold signing.** A BIP-39 seed still exists and is *reconstructed*
during recovery — unlike genuine MPC (ZenGo/Coinbase/Web3Auth-style) where a full key is never
assembled. Do not conflate the two. Telegram chose this deliberately: it keeps a standard
recoverable BIP-39 seed (TON-ecosystem-interoperable) while removing the user-facing seed-phrase
burden and avoiding a single custodial honeypot. The strategic goal is mass-market onboarding —
COO: self-custody with manual seeds is "exceedingly difficult" for normal people; CEO: "no need to
remember the seed phrase… this is how we simplify the whole thing."

**Takeaway model for DogTag:** offer a **low-friction, recoverable, self-custodial default** (no
raw seed UX), with manual seed export as an *advanced/optional* path — not the default.

Sources: <https://help.wallet.tg/article/86-security>, <https://wallet.tg/ton>,
<https://help.wallet.tg/article/4-ton-space>,
<https://cointelegraph.com/news/telegram-wallet-avoided-self-custody>,
<https://www.cryptopolitan.com/telegram-self-custody-wallet-for-americans/>,
<https://www.bitget.com/wiki/ton-space-wallet-telegram>.

## B2. DogTag's EVM equivalent — key gen, storage, balance, send/receive, WC

### B2.1 BIP39 → EVM address (the self-custody fallback path)

Identical flow both platforms: entropy → BIP39 mnemonic → 512-bit seed (PBKDF2) → BIP32 master →
BIP44 `m/44'/60'/0'/0/0` → secp256k1 keypair → Keccak-256(pubkey)[last 20 bytes] = EVM address.

**Android (web3j)** — `org.web3j:core`. Current line is **5.0.2** (Jan 2026) but it targets
**Java 21+**; on Android pin to **4.12.x** unless your AGP/desugaring toolchain meets Java 21.

```kotlin
import org.web3j.crypto.*
val entropy = ByteArray(16).also { java.security.SecureRandom().nextBytes(it) } // 12 words
val mnemonic = MnemonicUtils.generateMnemonic(entropy)
val seed = MnemonicUtils.generateSeed(mnemonic, "")
val master = Bip32ECKeyPair.generateKeyPair(seed)
val path = intArrayOf(
    44 or Bip32ECKeyPair.HARDENED_BIT, 60 or Bip32ECKeyPair.HARDENED_BIT,
    0 or Bip32ECKeyPair.HARDENED_BIT, 0, 0)              // m/44'/60'/0'/0/0
val credentials = Credentials.create(Bip32ECKeyPair.deriveKeyPair(master, path))
val address = credentials.address
```

(`zcash/kotlin-bip39` is a clean mnemonic-only alternative paired with web3j BIP32.)

**iOS (web3swift)** — `web3swift-team/web3swift` **3.3.2** (Sep 2025), via SPM, iOS 13+.

```swift
import web3swift; import Web3Core
let mnemonic = try BIP39.generateMnemonics(bitsOfEntropy: 128)!
let keystore = try BIP32Keystore(mnemonics: mnemonic, password: "",
    mnemonicsPassword: "", prefixPath: "m/44'/60'/0'/0")!
let address = keystore.addresses!.first!
```

**KMM caveat:** no mature dedicated KMM EVM HD-wallet SDK as of 2026 — keep derivation on each
platform's battle-tested lib (web3j / web3swift) behind a shared interface; don't bet a consumer
wallet on an immature KMM crypto lib.

### B2.2 Secure storage — *encrypt-then-store* (the must-get-right part)

**Neither the iOS Secure Enclave nor Android StrongBox/Keystore can store an arbitrary secret**
(a 256-bit seed / BIP39 mnemonic). The Secure Enclave only generates/holds its own EC P-256 keys.
Correct pattern:

> Generate a hardware key inside Secure Enclave / StrongBox → use it to **encrypt** the seed →
> store the **ciphertext** in normal storage (Keychain / EncryptedSharedPreferences / file).
> Decryption requires the hardware key, gated by biometrics/passcode.

**iOS:** create a P-256 key in the **Secure Enclave** (`kSecAttrTokenIDSecureEnclave`) with
`SecAccessControlCreateWithFlags([.privateKeyUsage, .biometryCurrentSet])`; encrypt the seed via
ECIES (`SecKeyCreateEncryptedData`, `.eciesEncryptionCofactorX963SHA256AESGCM`); store ciphertext
in the **Keychain** with `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`; gate use with
**LocalAuthentication** (`LAContext`, Face ID/Touch ID — enforced by the Enclave).

**Android:** generate an **AES-GCM key** in the Keystore with `setIsStrongBoxBacked(true)`
(API 28+, catch `StrongBoxUnavailableException` → retry without) and
`setUserAuthenticationRequired(true)`; encrypt the seed; store ciphertext in
**EncryptedSharedPreferences**/file; bind unlock to **BiometricPrompt** via a `CryptoObject`
wrapping the cipher.

**Checklist:** never log/serialize the plaintext seed; zero byte arrays after use;
`…ThisDeviceOnly`/no auto-backup; one-time mnemonic backup flow (the only true recovery if device
lost); root/jailbreak detection; require `biometryCurrentSet` so enrolling a new fingerprint/face
invalidates the key.

### B2.3 Seedless / MPC / embedded alternatives (the recommended default)

| Provider | Architecture | Native iOS/Android | Recovery | 2025/26 status |
|---|---|---|---|---|
| **MetaMask Embedded Wallets** (ex-Web3Auth) | MPC: **TSS + SSS**, key split across nodes/device/user, never reassembled | **Yes** native iOS + Android (+ RN/Flutter/Unity) | social/OAuth/device/password + optional seed factor | acquired by Consensys; docs now docs.metamask.io/embedded-wallets |
| **Privy** | non-custodial **2-of-3 Shamir SSS** + TEE; Privy can't sign alone | native iOS (Swift) + Android (Kotlin) | email/social/passkey-linked shares | acquired by **Stripe (Jun 2025)**; smoothest consumer onboarding |
| **Turnkey** | keys only inside hardware **TEEs**; verifiable, never exported | Swift/iOS + web, passkey-first | passkeys, email/SMS/OAuth recovery | enclave signing ~100–150ms |
| **Dynamic** | TSS-MPC shares | weaker native-mobile story historically | social/MFA | acquired by **Fireblocks (Oct 2025)** |
| **Coinbase embedded / Reown AppKit smart accounts** | ERC-4337/7702 smart accounts + embedded signer | AppKit covers iOS/Android | passkey/social | good if you also want gas sponsorship/AA |

MPC vs TSS: **TSS is an MPC technique** producing a single standard signature from distributed
shares (on-chain it looks like a normal EOA sig). **SSS (Shamir)** splits a secret for
backup/recovery but *reassembles* to sign — many providers use TSS for signing + SSS for recovery.

### B2.4 WalletConnect / Reown (in-app wallet acting as a wallet)

To let DogTag's wallet connect to external dApps (scan a `wc:` URI), use **Reown WalletKit**
(the wallet-side SDK; rebrand of WalletConnect). Legacy `WalletConnectSwiftV2` reached EOL
**2025-02-17** — use the Reown repos.

- **Android:** BOM ~**1.6.14** (Jun 2026): `platform("com.reown:android-bom:1.6.14")`,
  `com.reown:android-core`, `com.reown:walletkit`. Flow: init `CoreClient` + `WalletKit` →
  `WalletKit.pair(uri)` → `onSessionProposal` → `approveSession(namespaces)` → `onSessionRequest`
  (`eth_sendTransaction`, `personal_sign`) → sign with web3j credentials → `respondSessionRequest`.
- **iOS:** `reown-swift` (~1.x) via SPM. `WalletKit.instance.pair(uri:)` →
  `sessionProposalPublisher` → `approve(...)` → `sessionRequestPublisher` → sign → `respond(...)`.

### B2.5 Address / balance / send / receive

- **Balance:** `eth_getBalance` over the ROAX RPC. Android: `web3.ethGetBalance(addr, LATEST)`;
  iOS: `web3.eth.getBalance(for:)`. Format wei → PLASMA (`Convert.fromWei`). Configure the RPC to
  `https://devrpc.roax.net`, chainId 135.
- **Receive:** show the EVM address as text + QR — no on-chain action.
- **Send (self-custody path):** Android — `Transfer.sendFunds(...)` or build `RawTransaction` +
  `TransactionEncoder.signMessage(rawTx, 135L, credentials)` + `ethSendRawTransaction`. iOS — fill
  a `CodableTransaction`, attach keystore, `web3.eth.send(transaction)`. In the **MPC/embedded
  path** you skip local signing: hand the unsigned tx to the provider SDK
  (`signTransaction`/`sendTransaction`), which coordinates the threshold signature and returns the
  broadcast hash.

### B2.6 Recommendation

**Do NOT lead with raw seed phrases** for non-crypto pet owners — they cause support churn,
screenshot-leak risk, and permanent loss on device loss.

- **Default = embedded MPC wallet:** **MetaMask Embedded Wallets** (broadest native coverage) or
  **Privy** (smoothest onboarding). Both are non-custodial-with-caveats (TSS/SSS, provider can't
  sign alone), give email/social/passkey login with **no seed phrase**, and ship **native Swift +
  Kotlin** SDKs. (**Turnkey** if you want enclave-only, passkey-first, maximally verifiable
  custody.) This mirrors Telegram's "self-custody made to feel custodial-easy" — but with *real*
  MPC rather than Telegram's sharded-seed backup.
- **Optional advanced/export path = raw self-custody:** BIP39 via web3j/web3swift, derive
  `m/44'/60'/0'/0/0`, store with the **encrypt-then-store** Secure-Enclave/StrongBox + biometric
  pattern (B2.2). Satisfies crypto-natives and gives a true exit/ownership story.
- **dApp connectivity (all cases):** **Reown WalletKit** (Android `com.reown:walletkit`, iOS
  `reown-swift`).
- Pin & re-verify versions at build time (web3j 5.0.2 needs Java 21 → use 4.12.x on Android;
  web3swift 3.3.2; Reown Kotlin BOM ~1.6.14; reown-swift ~1.x).

---

## B3. Records "verifiable on chain when imported" — wallet ↔ on-chain identity

### B3.1 SBT ownership model — self-custodial address SHOULD own the DogTag SBT

Per `03-chain-contracts.md`, each pet has a non-transferable **`DogTagSBT`** (ERC-5192,
`tokenId` = canonical pet id), and credentials are merkle roots anchored on `DogTagIssuer`
contracts, with leaves bound to `tokenId`.

**Recommendation:** the pet's `DogTagSBT` should be **owned by the pet owner's self-custodial (or
embedded-MPC) address**, not a platform-custodial address. Rationale:

- It makes "this pet's identity belongs to *this* user" a **cryptographic, on-chain fact** the
  user controls — the whole point of self-custody, and the analogue of TON Space giving users
  their own keys.
- Because the SBT is **soulbound** (transfers revert), ownership can't be silently moved; binding
  it to the user's address makes the on-chain owner the user's verifiable anchor.
- Custodial ownership would re-introduce the counterparty trust that the self-custodial wallet
  exists to remove. (A custodial *option* — like Telegram's @wallet — can exist for users who
  refuse key management, but the verifiable-ownership story is strongest with self-custody.)

### B3.2 Import verification flow (no trust in the importer)

When a user imports a record (e.g. scans the QR/JWT from `04-custody-qr.md`, or pulls a wrapped
document), the app verifies it **against chain via the ROAX RPC**, exactly per `02-attestation.md`:

1. **Recompute the document hash** from the wrapped document's canonicalised data; confirm it
   equals `signature.targetHash`.
2. **Verify merkle membership:** `checkProof(proof, merkleRoot, targetHash)` (sorted-pair
   Keccak-256). For single-record credentials `proof` is empty and `merkleRoot === targetHash`.
3. **Check on-chain anchoring (RPC `eth_call`):** call the relevant `DogTagIssuer.isValid(merkleRoot)`
   (issued and not revoked). This is the network step that proves provenance.
4. **Bind to the pet's identity:** confirm the credential's leaf encodes the expected
   `tokenId`/`recordType` (`leaf = keccak256(abi.encode(tokenId, recordType, payloadHash))`), and
   read `DogTagSBT.ownerOf(tokenId)`.

### B3.3 How the wallet/address strengthens import verification

The wallet ties the user to their pets' on-chain identities and turns import from "trust the
sender" into "verify against chain + my own keys":

- **Ownership match:** the app checks `DogTagSBT.ownerOf(tokenId) == myWalletAddress`. A record
  imports as **"yours"** only if the on-chain SBT owner is the address the user controls — so a
  forged/stolen record for a pet you don't own won't bind to you.
- **Claim/transfer signing:** to *claim* a pet identity (e.g. at adoption/sale), the user signs a
  message/transaction with their self-custodial key. Because the SBT is soulbound, "transfer" is
  modelled as **admin/issuer burn-and-remint to the new owner's address** (or a claim function),
  authorised by a signature from the relevant party — never a plain `transferFrom`. The user's
  signature (EIP-191 `personal_sign` or EIP-712 typed data — see Alloy `sign_message` in
  `04-custody-qr.md`) proves control of the destination address.
- **Local + on-chain = strong:** steps B3.2(1–2) are offline cryptographic checks (no trust in the
  network); step (3) is the on-chain anchor; the **address match** in B3.3 is the personal anchor.
  Together: the credential is *authentic* (merkle), *issued and live* (RPC), *about this pet*
  (leaf binding), and *owned by this user* (SBT owner == wallet address). The self-custodial wallet
  is what supplies that last, user-controlled link — which is exactly why the SBT should live at
  the user's own address.

---

## Sources

**Part A (web stack):**
- wagmi createConfig — <https://wagmi.sh/react/api/createConfig>
- wagmi viem guide — <https://wagmi.sh/react/guides/viem>
- wagmi v1→v2 migration — <https://wagmi.sh/react/guides/migrate-from-v1-to-v2>
- wagmi useSwitchChain — <https://wagmi.sh/react/api/hooks/useSwitchChain>
- wagmi useWriteContract — <https://wagmi.sh/react/api/hooks/useWriteContract>
- wagmi useSendTransaction — <https://wagmi.sh/react/api/hooks/useSendTransaction>
- wagmi write-to-contract — <https://wagmi.sh/react/guides/write-to-contract>
- EIP-6963 multi-wallet (wagmi v2) — <https://dev.to/grimicorn/connecting-wallets-the-right-way-wagmi-v2-and-eip-6963-4k02>
- Reown AppKit React installation — <https://docs.reown.com/appkit/react/core/installation>
- Reown dashboard (projectId) — <https://dashboard.reown.com>
- EIP-1193 (provider interface) — <https://eips.ethereum.org/EIPS/eip-1193>
- EIP-3085 (wallet_addEthereumChain) — <https://eips.ethereum.org/EIPS/eip-3085>
- viem chains / defineChain — <https://viem.sh/docs/chains/introduction>
- viem encodeFunctionData — <https://viem.sh/docs/contract/encodeFunctionData>
- (cross-check MetaMask edge cases) — <https://docs.metamask.io>

**Part B (Telegram + mobile):**
- Telegram Wallet security (BIP-39, sharded backup) — <https://help.wallet.tg/article/86-security>
- TON Space self-custodial page — <https://wallet.tg/ton>
- TON Space / DeFi Account help — <https://help.wallet.tg/article/4-ton-space>
- Telegram custody-by-default (COO) — <https://cointelegraph.com/news/telegram-wallet-avoided-self-custody>
- US launch + CEO "split-key" quote — <https://www.cryptopolitan.com/telegram-self-custody-wallet-for-americans/>
- UX/onboarding, TON Connect — <https://www.bitget.com/wiki/ton-space-wallet-telegram>
- US launch coverage — <https://www.cnbc.com/2025/07/22/telegram-crypto-wallet-us.html>
- web3j Bip44WalletUtils — <https://github.com/LFDT-web3j/web3j/blob/main/core/src/main/java/org/web3j/crypto/Bip44WalletUtils.java>
- web3j MnemonicUtils — <https://github.com/LFDT-web3j/web3j/blob/main/crypto/src/main/java/org/web3j/crypto/MnemonicUtils.java>
- web3j 5.0.2 release notes — <https://blog.web3labs.com/web3j-5-0-2-a-community-release-that-moves-us-forward/>
- kotlin-bip39 — <https://github.com/zcash/kotlin-bip39>
- web3swift — <https://github.com/web3swift-team/web3swift>
- web3.swift (Swift Package Index) — <https://swiftpackageindex.com/argentlabs/web3.swift>
- Apple — The Secure Enclave — <https://support.apple.com/guide/security/the-secure-enclave-sec59b0b31ff/web>
- Android Keystore — <https://developer.android.com/privacy-and-security/keystore>
- Android Hardware-backed Keystore — <https://source.android.com/docs/security/features/keystore>
- iOS Secure Enclave explained — <https://ptkd.com/journal/ios-secure-enclave-explained>
- StrongBox vs hardware-backed — <https://proandroiddev.com/android-keystore-what-is-the-difference-between-strongbox-and-hardware-backed-keys-4c276ea78fd0>
- KeyDroid key-storage analysis — <https://arxiv.org/html/2507.07927v1>
- MetaMask Embedded Wallets MPC architecture — <https://docs.metamask.io/embedded-wallets/infrastructure/mpc-architecture/>
- MetaMask Embedded Wallets announcement — <https://metamask.io/news/metamask-embedded-wallets-frictionless-web3-onboarding-built-in>
- Web3Auth WaaS comparison — <https://blog.web3auth.io/waas-wallet-comparison/>
- How Privy embedded wallets work — <https://privy.io/blog/how-privy-embedded-wallets-work>
- Openfort — top embedded wallets — <https://www.openfort.io/blog/top-10-embedded-wallets>
- Reown WalletKit overview — <https://docs.reown.com/walletkit/overview>
- reown-kotlin — <https://github.com/reown-com/reown-kotlin>
- reown-swift — <https://github.com/reown-com/reown-swift>
- Reown WalletKit iOS installation — <https://docs.reown.com/walletkit/ios/installation>

**Existing system docs cross-referenced:** `docs/research/02-attestation.md`,
`docs/research/03-chain-contracts.md`, `docs/research/04-custody-qr.md`.

# 03 — Chain & Contracts Research: DogTag Pet-Credentialing on ROAX

> Target chain: **ROAX** — chainId `0x87` (`135` decimal), RPC `https://devrpc.roax.net`,
> native gas token **PLASMA**, Blockscout-style explorer `https://explorer.roax.net`.
> Toolchain: **Foundry** + **OpenZeppelin Contracts v5.x**.
> Date: 2026-06-17.

This document gives concrete, EVM-accurate guidance for the DogTag system: a
non-transferable on-chain identity token per pet ("chip"), a factory that deploys
per-record-type issuer contracts, role-based whitelisting of issuer signers, and a
merkle-root anchoring scheme for credentials. All snippets target Solidity `^0.8.20`
(OZ v5 requirement) and OpenZeppelin Contracts **v5**.

---

## 1. Soulbound Token (SBT) standard — ERC-5192 + non-transferable ERC-721

### 1.1 ERC-5192 (minimal soulbound interface)

ERC-5192 is the minimal standard for "locked" (account-bound) NFTs. It adds a single
view function and two events on top of ERC-721. The full interface
(source: <https://eips.ethereum.org/EIPS/eip-5192>):

```solidity
// SPDX-License-Identifier: CC0-1.0
pragma solidity ^0.8.20;

interface IERC5192 {
    /// @notice Emitted when the locking status is changed to locked.
    /// @dev If a token is minted and the status is locked, this event should be emitted.
    event Locked(uint256 tokenId);

    /// @notice Emitted when the locking status is changed to unlocked.
    event Unlocked(uint256 tokenId);

    /// @notice Returns the locking status of a Soulbound Token.
    /// @dev SBTs assigned to the zero address are considered invalid; calls
    ///      querying about them throw.
    /// @param tokenId The identifier for an SBT.
    function locked(uint256 tokenId) external view returns (bool);
}
```

- **ERC-165 interface id:** `0xb45a3c0e` — `supportsInterface(0xb45a3c0e)` MUST return `true`.
- **Spec rule:** *every* ERC-721 function that transfers a token (`transferFrom`,
  `safeTransferFrom`) MUST `revert` while `locked(tokenId) == true`.
- For a permanently-bound DogTag, `locked()` always returns `true` and we emit
  `Locked(tokenId)` at mint. We never emit `Unlocked`.

### 1.2 Non-transferable ERC-721 in OpenZeppelin v5 — override `_update`

**Key v5 change:** OZ v5 removed `_beforeTokenTransfer` / `_afterTokenTransfer`. All
mint/transfer/burn state changes now funnel through a single internal hook:

```solidity
function _update(address to, uint256 tokenId, address auth)
    internal
    virtual
    returns (address);   // returns the PREVIOUS owner (`from`)
```

(source: <https://docs.openzeppelin.com/contracts/5.x/api/token/erc721#ERC721-_update-address-uint256-address->)

- `_update` "transfers `tokenId` to `to`, or mints if the current owner is the zero
  address, or burns if `to` is the zero address. Returns the owner of `tokenId`
  before the update."
- Mint is detected by `from == address(0)` (previous owner is zero).
- Burn is detected by `to == address(0)`.
- A pure transfer is `from != address(0) && to != address(0)`.

**Soulbound implementation** — allow mint (and optionally burn by admin), block transfers:

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {ERC721} from "@openzeppelin/contracts/token/ERC721/ERC721.sol";
import {IERC165} from "@openzeppelin/contracts/utils/introspection/IERC165.sol";

interface IERC5192 {
    event Locked(uint256 tokenId);
    event Unlocked(uint256 tokenId);
    function locked(uint256 tokenId) external view returns (bool);
}

abstract contract ERC721Soulbound is ERC721, IERC5192 {
    error Soulbound();                  // cheaper than string revert

    function locked(uint256 tokenId) public view virtual returns (bool) {
        _requireOwned(tokenId);         // OZ v5 helper: reverts if not minted
        return true;                    // DogTags are permanently bound
    }

    /// @dev Single override point in OZ v5. Block transfers; permit mint & burn.
    function _update(address to, uint256 tokenId, address auth)
        internal
        virtual
        override
        returns (address)
    {
        address from = _ownerOf(tokenId);
        // from == 0 -> mint (allowed); to == 0 -> burn (allowed)
        if (from != address(0) && to != address(0)) revert Soulbound();
        return super._update(to, tokenId, auth);
    }

    function supportsInterface(bytes4 interfaceId)
        public
        view
        virtual
        override(ERC721)
        returns (bool)
    {
        return interfaceId == 0xb45a3c0e // IERC5192
            || super.supportsInterface(interfaceId);
    }
}
```

Notes:
- Overriding `_update` (not `transferFrom`) is the v5-correct choice: it catches
  `transferFrom`, both `safeTransferFrom` overloads, and any future path in one place.
  (Forum confirmation of the v5 migration: <https://forum.openzeppelin.com/t/soulbound-nfts-in-erc721-version-5-0/41550>)
- Emit `Locked(tokenId)` inside your `mint` function right after `_safeMint`, per
  ERC-5192's "emit at mint if locked" rule.
- Approvals are harmless on a soulbound token (transfers revert anyway), but you may
  also override `approve` / `setApprovalForAll` to revert for cleanliness.

### 1.3 DogTag token design

The DogTag is one contract — `DogTagSBT` (the on-chain pet identity). `tokenId` is the
stable, universally-referenced pet identifier. Recommended `tokenId` strategy: a
monotonic counter (`uint256 _nextId`) so ids are dense and cheap, with the chip/microchip
number stored off-chain or in a `mapping(uint256 => bytes32) chipHash`. Avoid deriving
`tokenId` from the raw microchip number (privacy + collision concerns).

---

## 2. Factory + per-record-type issuer pattern (OpenAttestation-style)

### 2.1 Reference pattern — OpenAttestation DocumentStore / DocumentStoreCreator

OpenAttestation uses a `DocumentStoreCreator` factory that deploys a `DocumentStore`
per issuing organisation. Each `DocumentStore` anchors document hashes / merkle roots
and supports issue + revoke. (Docs: <https://www.openattestation.com/docs/lib-section/remote-files/document-store>,
repo: <https://github.com/Open-Attestation/document-store>.) Revocation can target either
a single document's `targetHash` or a batch `merkleRoot` — exactly the dual-mode we want
in §4.

For DogTag, we generalise this: a **`DogTagIssuerFactory`** deploys a
**`DogTagIssuer`** per record type (e.g. `VACCINATION`, `OWNERSHIP`, `LICENSE`,
`HEALTH_CERT`). Each issuer is an independent anchoring contract scoped to one record
type, governed by the central registry (§3).

### 2.2 Minimal-proxy (EIP-1167 clones) vs full deploy

OpenZeppelin's `Clones` library implements EIP-1167. A clone is a ~45-byte runtime proxy
that `delegatecall`s a fixed implementation address.
(source: <https://github.com/OpenZeppelin/openzeppelin-contracts/blob/master/contracts/proxy/Clones.sol>)

| | **EIP-1167 clone** | **Full deploy** |
|---|---|---|
| Deploy gas per issuer | ~**40k–55k** (deploys ~45 bytes) | full creation cost (often **1–3M+** for a non-trivial contract) |
| Code per instance | shares one implementation | independent bytecode each time |
| Bytecode immutability | implementation fixed at clone time | independent |
| Constructor | **none** — must use an `initialize()` function | normal constructor |
| Per-call overhead | one extra `delegatecall` (~2.6k gas warm) | none |
| Verification on explorer | verify implementation once; clones show as proxies | verify each contract |
| Upgradeability | clones are *not* upgradeable (impl address is baked into the 1167 bytecode) | n/a |

**Decision for DogTag: use EIP-1167 clones via `Clones.clone` / `cloneDeterministic`.**
Rationale: we expect a bounded but growing set of issuer contracts (one per record type,
and potentially per-jurisdiction), all sharing identical logic. Cloning gives ~95% deploy
gas savings, one-time implementation verification, and deterministic addresses
(`cloneDeterministic` + `predictDeterministicAddress`) so the off-chain indexer can
pre-compute an issuer address from `(recordType)` salt. The per-call `delegatecall`
overhead is negligible for issue/revoke. We give up per-clone upgradeability — acceptable
because issuers are intentionally simple and immutable; if logic must change we deploy a
new implementation and clone forward.

### 2.3 Factory + clone code

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Clones} from "@openzeppelin/contracts/proxy/Clones.sol";

interface IDogTagIssuer {
    function initialize(bytes32 recordType, address registry, address admin) external;
}

contract DogTagIssuerFactory {
    using Clones for address;

    address public immutable implementation;       // the DogTagIssuer logic contract
    address public immutable registry;              // central IssuerRegistry (§3)

    event IssuerDeployed(bytes32 indexed recordType, address issuer);

    constructor(address implementation_, address registry_) {
        implementation = implementation_;
        registry = registry_;
    }

    /// @dev Deterministic: address = f(implementation, recordType). One issuer per type.
    function deployIssuer(bytes32 recordType, address admin)
        external
        returns (address issuer)
    {
        issuer = implementation.cloneDeterministic(recordType); // salt = recordType
        IDogTagIssuer(issuer).initialize(recordType, registry, admin);
        emit IssuerDeployed(recordType, issuer);
    }

    function predictIssuer(bytes32 recordType) external view returns (address) {
        return implementation.predictDeterministicAddress(recordType, address(this));
    }
}
```

Because clones have no constructor, `DogTagIssuer` must use an `initialize()` guarded
against re-init (e.g. a boolean flag, or OZ's `Initializable`). Use
`@openzeppelin/contracts-upgradeable` base classes for clone targets if you want OZ's
`Initializable` machinery; otherwise a one-line `require(!_initialized)` is sufficient
for a non-upgradeable clone.

---

## 3. Access control / whitelist — AccessControl + IssuerRegistry

### 3.1 AccessControl vs Ownable (OZ v5)

(source: <https://docs.openzeppelin.com/contracts/5.x/access-control>)

- **`Ownable`** — single owner, all-or-nothing. `constructor(address initialOwner)`,
  `onlyOwner`, `transferOwnership`, `renounceOwnership`. Good for tiny contracts.
- **`AccessControl`** — role-based, principle of least privilege. We use this for both
  the registry and the issuers because we need *multiple* whitelisted signing addresses
  and a separate protocol admin.

Roles in OZ v5 `AccessControl`:

```solidity
bytes32 public constant ISSUER_ROLE = keccak256("ISSUER_ROLE");
// DEFAULT_ADMIN_ROLE == bytes32(0) — admin of every role by default.

constructor(address protocolAdmin) {
    _grantRole(DEFAULT_ADMIN_ROLE, protocolAdmin);   // can grant/revoke all roles
}

function doThing() external onlyRole(ISSUER_ROLE) { ... }
```

Key functions: `hasRole(role, account)`, `grantRole(role, account)` (caller must hold the
role's admin role), `revokeRole(role, account)`, `renounceRole(role, callerConfirmation)`,
`getRoleAdmin(role)`, internal `_grantRole` / `_revokeRole`, and `_setRoleAdmin(role,
adminRole)` to make one role the admin of another. Base `AccessControl` does **not**
enumerate members on-chain — use `AccessControlEnumerable` or index `RoleGranted` /
`RoleRevoked` events off-chain.

### 3.2 Central IssuerRegistry (protocol-admin whitelist of signing addresses)

The protocol admin whitelists issuer **signing addresses** centrally. Issuer contracts
gate issue/revoke by consulting the registry, so revoking a compromised signer at the
registry instantly disables it across *all* issuers.

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {AccessControl} from "@openzeppelin/contracts/access/AccessControl.sol";

/// Central whitelist. Protocol admin holds DEFAULT_ADMIN_ROLE.
contract IssuerRegistry is AccessControl {
    bytes32 public constant ISSUER_ROLE = keccak256("ISSUER_ROLE");

    constructor(address protocolAdmin) {
        _grantRole(DEFAULT_ADMIN_ROLE, protocolAdmin);
    }

    // Admin-only because ISSUER_ROLE's admin defaults to DEFAULT_ADMIN_ROLE.
    function whitelistIssuer(address signer)   external { grantRole(ISSUER_ROLE, signer); }
    function delistIssuer(address signer)      external { revokeRole(ISSUER_ROLE, signer); }

    function isWhitelisted(address signer) external view returns (bool) {
        return hasRole(ISSUER_ROLE, signer);
    }
}
```

### 3.3 Issuer gated by the registry

```solidity
// inside DogTagIssuer
IssuerRegistry public registry;

modifier onlyWhitelisted() {
    require(registry.isWhitelisted(msg.sender), "not a whitelisted issuer");
    _;
}
```

Design choice: a **single source of truth** (registry) for who may sign, rather than
replicating `ISSUER_ROLE` grants into each issuer clone. This keeps clones stateless
w.r.t. authorization and makes global revocation O(1). If you want per-record-type signer
scoping, store `mapping(bytes32 recordType => mapping(address => bool))` in the registry
and check `registry.isWhitelistedFor(recordType, msg.sender)`.

---

## 4. Anchoring a merkle root — issue / revoke / view

### 4.1 Pattern (matches OpenAttestation DocumentStore semantics)

Two `mapping(bytes32 => uint256)` storing the **block timestamp** of issuance/revocation
(`0` == never). Storing a timestamp rather than a bool gives a free audit trail and an
idempotency guard at no extra storage cost. (OpenAttestation uses analogous
`documentIssued` / `documentRevoked` maps and `DocumentIssued` / `DocumentRevoked`
events — <https://www.openattestation.com/docs/did-section/revoke-document-did/revoke-using-document-store>.)

```solidity
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

import {Initializable} from "@openzeppelin/contracts-upgradeable/proxy/utils/Initializable.sol";

interface IIssuerRegistry { function isWhitelisted(address) external view returns (bool); }

contract DogTagIssuer is Initializable {
    bytes32 public recordType;
    IIssuerRegistry public registry;
    address public admin;

    /// root => unix timestamp issued (0 = not issued)
    mapping(bytes32 => uint256) public issuedAt;
    /// root => unix timestamp revoked (0 = not revoked)
    mapping(bytes32 => uint256) public revokedAt;

    event RootIssued(bytes32 indexed root, address indexed issuer, uint256 timestamp);
    event RootRevoked(bytes32 indexed root, address indexed issuer, uint256 timestamp);

    modifier onlyWhitelisted() {
        require(registry.isWhitelisted(msg.sender), "DogTag: not whitelisted");
        _;
    }

    function initialize(bytes32 recordType_, address registry_, address admin_)
        external
        initializer
    {
        recordType = recordType_;
        registry = IIssuerRegistry(registry_);
        admin = admin_;
    }

    // --- mutating ---
    function issue(bytes32 root) public onlyWhitelisted {
        require(root != bytes32(0), "zero root");
        require(issuedAt[root] == 0, "already issued");
        issuedAt[root] = block.timestamp;
        emit RootIssued(root, msg.sender, block.timestamp);
    }

    function revoke(bytes32 root) public onlyWhitelisted {
        require(revokedAt[root] == 0, "already revoked");
        revokedAt[root] = block.timestamp;
        emit RootRevoked(root, msg.sender, block.timestamp);
    }

    // Batch convenience — same proof format, just many roots.
    function bulkIssue(bytes32[] calldata roots) external onlyWhitelisted {
        for (uint256 i; i < roots.length; ++i) issue(roots[i]);
    }
    function bulkRevoke(bytes32[] calldata roots) external onlyWhitelisted {
        for (uint256 i; i < roots.length; ++i) revoke(roots[i]);
    }

    // --- views ---
    function isIssued(bytes32 root) external view returns (bool) {
        return issuedAt[root] != 0;
    }
    function isRevoked(bytes32 root) external view returns (bool) {
        return revokedAt[root] != 0;
    }
    /// A credential is valid iff its root is issued and not revoked.
    function isValid(bytes32 root) external view returns (bool) {
        return issuedAt[root] != 0 && revokedAt[root] == 0;
    }
}
```

### 4.2 Single-record now, batched-roots later — without breaking the proof format

The anchored value is always a **merkle root `bytes32`**. The verifier always checks
`isValid(root)`. The trick is how `root` relates to a credential:

- **Single-record (today):** a credential's merkle tree has exactly one leaf. By
  convention, **`root == leafHash == documentHash`** — i.e. the proof is empty
  (`proof.length == 0`) and the verifier treats the document hash as the root. Nothing
  special on-chain.
- **Batched (later):** many credentials in one tree. Each credential carries
  `{ documentHash (leaf), proof[] }`; the verifier computes
  `computedRoot = processProof(proof, leaf)` and checks `isValid(computedRoot)`. The
  on-chain contract is **unchanged** — it still only stores/queries roots.

Because the on-chain interface is root-only and the single-record case is just "empty
proof, root == leaf," moving to batches requires **no contract change and no migration**:
old single-record credentials keep verifying (empty proof), new ones ship a proof. Use
OZ's `MerkleProof.processProof` / `verify` off-chain or on-chain as needed
(`@openzeppelin/contracts/utils/cryptography/MerkleProof.sol`), with sorted-pair hashing
so leaf ordering is irrelevant — the standard, OpenZeppelin-compatible scheme.

> Tie-in to §1: the credential leaf should bind the DogTag `tokenId` (e.g.
> `leaf = keccak256(abi.encode(tokenId, recordType, payloadHash))`) so a credential is
> cryptographically attached to a specific pet identity.

---

## 5. Foundry — deploy to a custom chain + Blockscout verification

### 5.1 `foundry.toml`

```toml
[profile.default]
src = "src"
out = "out"
libs = ["lib"]
solc_version = "0.8.24"
evm_version = "shanghai"     # see §6 — conservative for an unknown EVM chain
optimizer = true
optimizer_runs = 200

[rpc_endpoints]
roax = "https://devrpc.roax.net"

# Lets `--verifier blockscout` resolve a URL by chain or alias if desired.
[etherscan]
roax = { key = "any", url = "https://explorer.roax.net/api/", chain = 135 }
```

`--chain` / `block.chainid` is **135** (`0x87`). Note `--chain` only sets the chain id; it
does **not** pick an RPC — always pass `--rpc-url`.
(source: <https://getfoundry.sh/forge/deploying>, <https://getfoundry.sh/reference/cli/forge/create>)

### 5.2 Deploy — `forge create`

```bash
# Implementation + factory + registry, one at a time.
forge create \
  --rpc-url https://devrpc.roax.net \
  --chain 135 \
  --private-key $PRIVATE_KEY \
  --broadcast \
  src/DogTagIssuer.sol:DogTagIssuer

# With constructor args (e.g. IssuerRegistry(protocolAdmin)):
forge create \
  --rpc-url https://devrpc.roax.net \
  --chain 135 \
  --private-key $PRIVATE_KEY \
  --broadcast \
  src/IssuerRegistry.sol:IssuerRegistry \
  --constructor-args 0xYourProtocolAdmin
```

> If ROAX's RPC does not serve EIP-1559 fee data ("Failed to get EIP-1559 fees"),
> append `--legacy` (and optionally `--gas-price <wei>`). See §6.
> (source: <https://github.com/foundry-rs/foundry/issues/8047>)

### 5.3 Deploy — `forge script` (preferred for multi-contract wiring)

```solidity
// script/Deploy.s.sol
// forge-std Script that deploys registry, impl, factory and wires roles.
import {Script} from "forge-std/Script.sol";
// ...deploy IssuerRegistry, DogTagIssuer (impl), DogTagIssuerFactory, DogTagSBT...
```

```bash
forge script script/Deploy.s.sol:Deploy \
  --rpc-url https://devrpc.roax.net \
  --chain 135 \
  --private-key $PRIVATE_KEY \
  --broadcast \
  -vvvv
# add --legacy if the chain lacks EIP-1559
```

### 5.4 Verify on Blockscout

(source: <https://docs.blockscout.com/devs/verification/foundry-verification>,
<https://getfoundry.sh/forge/reference/forge-verify-contract/>)

```bash
forge verify-contract \
  --rpc-url https://devrpc.roax.net \
  --verifier blockscout \
  --verifier-url https://explorer.roax.net/api/ \
  <DEPLOYED_ADDRESS> \
  src/DogTagIssuer.sol:DogTagIssuer

# If the contract has constructor args, ABI-encode and pass them:
forge verify-contract \
  --rpc-url https://devrpc.roax.net \
  --verifier blockscout \
  --verifier-url https://explorer.roax.net/api/ \
  --constructor-args $(cast abi-encode "constructor(address)" 0xYourProtocolAdmin) \
  <DEPLOYED_ADDRESS> \
  src/IssuerRegistry.sol:IssuerRegistry
```

Notes:
- Blockscout verification needs the **`/api/`** suffix on the verifier URL; an API key is
  optional for Blockscout. Some Blockscout versions want `/api?` — try `/api/` first.
- To verify during deployment add `--verify --verifier blockscout --verifier-url
  https://explorer.roax.net/api/` to `forge create` / `forge script`.
- **Clones:** verify the *implementation* once. EIP-1167 clones appear as proxies that
  Blockscout can link to the verified implementation; you do not re-verify each clone.

---

## 6. Gas / EVM-compatibility for an unknown EVM chain

### 6.1 Pin a conservative `evm_version`

The biggest footgun on a less-current EVM chain is the **`PUSH0`** opcode (EIP-3855),
emitted by solc when `evm_version >= shanghai`. If ROAX's EVM predates Shanghai, `PUSH0`
causes deploy/runtime reverts (`invalid opcode`).

- **Safest:** set `evm_version = "paris"` (a.k.a. "merge") in `foundry.toml`. solc then
  avoids `PUSH0`, producing bytecode valid on any London/Merge-era EVM.
- If you have confirmed ROAX supports Shanghai, `evm_version = "shanghai"` is fine (the
  §5.1 default). When unsure, **drop to `paris`/`london`**.
- Pinning matters because Foundry does **not** validate evm_version against solc version,
  and auto-detected solc ignores it. Always pin both `solc_version` and `evm_version`.
  (sources: <https://www.getfoundry.sh/config/reference/solidity-compiler>,
  <https://github.com/foundry-rs/foundry/issues/6943>)

Recommendation for DogTag on ROAX: start with **`evm_version = "paris"`** until Shanghai
support is verified on `devrpc.roax.net`, then optionally bump to `shanghai`. Do a smoke
test: deploy a trivial contract that uses `PUSH0`-eligible code and confirm it executes.

### 6.2 EIP-1559 vs legacy transactions

- Foundry defaults to **EIP-1559** (type-2) txs. If the chain's RPC doesn't return
  `eth_feeHistory` / base-fee data, you get **"Failed to get EIP-1559 fees."**
- Fix: pass **`--legacy`** to use type-0 txs (single `gasPrice`), optionally with
  `--gas-price <wei>` and `--gas-limit <n>`. Add `--legacy` to `forge create`,
  `forge script`, and `cast send`.
  (sources: <https://getfoundry.sh/forge/deploying/>,
  <https://github.com/foundry-rs/foundry/issues/8047>)
- The native gas token being **PLASMA** (not ETH) changes nothing for Foundry: gas is
  still paid in the chain's native token; fund the deployer with PLASMA and quote
  `--gas-price` in PLASMA wei. Foundry has no concept of the token name.

### 6.3 General compatibility checklist for ROAX

- [ ] Confirm chainId via `cast chain-id --rpc-url https://devrpc.roax.net` → expect `135`.
- [ ] Confirm fee model: `cast block latest --rpc-url ... | grep baseFeePerGas`. Absent →
      use `--legacy`.
- [ ] Confirm hardfork / `PUSH0` support with a smoke-deploy; otherwise set
      `evm_version = "paris"`.
- [ ] Confirm `CREATE2` works (needed for `cloneDeterministic`) — universal on EVM ≥
      Constantinople, so safe on any London/Shanghai chain.
- [ ] Keep `optimizer_runs` modest (200) unless contract-size limits bite.

---

## Recommended contract set (summary)

| Contract | Responsibility |
|---|---|
| `DogTagSBT` | Non-transferable ERC-721 (ERC-5192) — one token per pet; `tokenId` is the canonical pet id. |
| `IssuerRegistry` | Central `AccessControl` whitelist of issuer signing addresses; `DEFAULT_ADMIN_ROLE` = protocol admin, `ISSUER_ROLE` = whitelisted signers. |
| `DogTagIssuer` (implementation) | Per-record-type anchoring contract: `issue`/`revoke` merkle roots, gated by `IssuerRegistry`. Deployed as EIP-1167 clones. |
| `DogTagIssuerFactory` | Deploys `DogTagIssuer` clones (`cloneDeterministic`, salt = recordType) and initializes them. |

---

## Sources

- ERC-5192: <https://eips.ethereum.org/EIPS/eip-5192>
- OZ v5 ERC721 `_update`: <https://docs.openzeppelin.com/contracts/5.x/api/token/erc721>
- OZ v5 soulbound migration (forum): <https://forum.openzeppelin.com/t/soulbound-nfts-in-erc721-version-5-0/41550>
- OZ Clones (EIP-1167): <https://github.com/OpenZeppelin/openzeppelin-contracts/blob/master/contracts/proxy/Clones.sol>
- OZ v5 Access Control: <https://docs.openzeppelin.com/contracts/5.x/access-control>
- OpenAttestation Document Store: <https://www.openattestation.com/docs/lib-section/remote-files/document-store>
- OpenAttestation revoke (issue/revoke + merkleRoot/targetHash): <https://www.openattestation.com/docs/did-section/revoke-document-did/revoke-using-document-store>
- Foundry deploying: <https://getfoundry.sh/forge/deploying>
- `forge create` reference: <https://getfoundry.sh/reference/cli/forge/create>
- `forge verify-contract` reference: <https://getfoundry.sh/forge/reference/forge-verify-contract/>
- Blockscout Foundry verification: <https://docs.blockscout.com/devs/verification/foundry-verification>
- Foundry solidity-compiler config (evm_version): <https://www.getfoundry.sh/config/reference/solidity-compiler>
- PUSH0 / evm_version issue: <https://github.com/foundry-rs/foundry/issues/6943>
- EIP-1559 fees / `--legacy`: <https://github.com/foundry-rs/foundry/issues/8047>

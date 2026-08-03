// Mock chain state for the consent-history e2e: two owner-blind `Verified` logs on Rex's tag
// (handle 424242) plus their `recordVerificationZK` transactions.
//
// The bytes are REAL ABI encodings, not hand-rolled: generated with viem `encodeEventTopics` +
// `encodeAbiParameters` (event) and `encodeFunctionData` (calldata) over the exact contract ABIs,
// then round-tripped through viem's decoders to confirm the app-extracted values (dogTagId,
// purpose, nullifier, deadline, ts, and pub[5] recordType) before being frozen here. Regenerate by
// re-running that script if the event or calldata shape ever changes (it is frozen with the M3 VK).
//
//   dogTagId       = fieldOfValue(Integer("424242"))  - Rex's canonical on-chain tag id
//   log 1 (0xd1…)  = purpose boarding_intake, recordType VACCINATION,
//                    ts 1751000000 (2025-06-27), deadline 4102444800 (2100) → window OPEN
//   log 2 (0xd2…)  = purpose travel_check, recordType VACCINATION,
//                    ts 1749900000 (2025-06-14), deadline 1750000000 (2025-06-15) → window CLOSED

/** topics[1] of both logs - the app must query exactly this held-tag id. */
export const REX_DOGTAGID_TOPIC = "0x2aa145640869b118466a6f897b37241ea667694826ac38a8ce7bdcba5ad1f198";

export const OPEN_TX_HASH = "0x" + "d1".repeat(32);
export const CLOSED_TX_HASH = "0x" + "d2".repeat(32);
export const OPEN_NULLIFIER = "0x" + "aa".repeat(31) + "01";
export const RELAYER = "0x1111111111111111111111111111111111111111";

const REGISTRY = "0x2b4d6f8a0c1e3a5b7d9f0e2c4a6b8d0f1e3a5c70";
const VERIFIED_TOPIC0 = "0xeb5f75f2727c9d0d91b1fe9120ed35d6867952fa5f5103488f44a7afe21f532b";
const RELAYER_TOPIC = "0x0000000000000000000000001111111111111111111111111111111111111111";
const BLOCK_HASH = "0x" + "e0".repeat(32);

export const VERIFIED_LOGS = [
  {
    address: REGISTRY,
    topics: [VERIFIED_TOPIC0, REX_DOGTAGID_TOPIC, RELAYER_TOPIC],
    data:
      "0x26e532b6f5b66521fea3ea9a33d09f9b6cc6ca45a765074832784d636e02ba01" +
      "bb".repeat(31) + "02" +
      "00000000000000000000000000000000000000000000000000000000684ee180" +
      "00000000000000000000000000000000000000000000000000000000684d5ae0",
    blockNumber: "0x30bd8",
    transactionHash: CLOSED_TX_HASH,
    transactionIndex: "0x0",
    blockHash: BLOCK_HASH,
    logIndex: "0x0",
    removed: false,
  },
  {
    address: REGISTRY,
    topics: [VERIFIED_TOPIC0, REX_DOGTAGID_TOPIC, RELAYER_TOPIC],
    data:
      "0x0d35de973921c6fca6d7ad626fe13c4017a093733a6a21689b631b2c61b1c18d" +
      "aa".repeat(31) + "01" +
      "00000000000000000000000000000000000000000000000000000000f4865700" +
      "00000000000000000000000000000000000000000000000000000000685e23c0",
    blockNumber: "0x30d40",
    transactionHash: OPEN_TX_HASH,
    transactionIndex: "0x0",
    blockHash: BLOCK_HASH,
    logIndex: "0x0",
    removed: false,
  },
];

/** Shared `recordVerificationZK(a,b,c,pub[7])` calldata prefix: selector + junk proof points (the
 *  renderer never validates them). Each tx appends its own pub[7] tail. */
const CALLDATA_PREFIX =
  "0xdd080593" +
  "0000000000000000000000000000000000000000000000000000000000000001" +
  "0000000000000000000000000000000000000000000000000000000000000002" +
  "0000000000000000000000000000000000000000000000000000000000000003" +
  "0000000000000000000000000000000000000000000000000000000000000004" +
  "0000000000000000000000000000000000000000000000000000000000000005" +
  "0000000000000000000000000000000000000000000000000000000000000006" +
  "0000000000000000000000000000000000000000000000000000000000000007" +
  "0000000000000000000000000000000000000000000000000000000000000008";

const VACCINATION_FIELD = "0447dc2457dac487b61ce87d5f44372e8e93555a4f0f15e75ada01aed8ca055f";

function tx(hash: string, blockNumber: string, pubTail: string) {
  return {
    hash,
    input: CALLDATA_PREFIX + pubTail,
    blockNumber,
    blockHash: BLOCK_HASH,
    transactionIndex: "0x0",
    from: RELAYER,
    to: REGISTRY,
    value: "0x0",
    gas: "0x7a120",
    gasPrice: "0x1",
    nonce: "0x1",
    type: "0x0",
    v: "0x0",
    r: "0x1",
    s: "0x1",
  };
}

export const CONSENT_TXS: Record<string, unknown> = {
  [OPEN_TX_HASH]: tx(
    OPEN_TX_HASH,
    "0x30d40",
    // pub = [dogTagId, purpose, relayer, nullifier, R, recordType, deadline]
    REX_DOGTAGID_TOPIC.slice(2) +
      "0d35de973921c6fca6d7ad626fe13c4017a093733a6a21689b631b2c61b1c18d" +
      RELAYER_TOPIC.slice(2) +
      "aa".repeat(31) + "01" +
      "0000000000000000000000000000000000000000000000000000000000000001" +
      VACCINATION_FIELD +
      "00000000000000000000000000000000000000000000000000000000f4865700",
  ),
  [CLOSED_TX_HASH]: tx(
    CLOSED_TX_HASH,
    "0x30bd8",
    REX_DOGTAGID_TOPIC.slice(2) +
      "26e532b6f5b66521fea3ea9a33d09f9b6cc6ca45a765074832784d636e02ba01" +
      RELAYER_TOPIC.slice(2) +
      "bb".repeat(31) + "02" +
      "0000000000000000000000000000000000000000000000000000000000000001" +
      VACCINATION_FIELD +
      "00000000000000000000000000000000000000000000000000000000684ee180",
  ),
};

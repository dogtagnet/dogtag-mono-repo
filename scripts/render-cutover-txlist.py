#!/usr/bin/env python3
"""Render the rehearsal's broadcast into the committed transaction list.

`contracts/broadcast/` is gitignored, so the raw `run-latest.json` cannot itself be the deliverable.
This turns it into `docs/CUTOVER_TRANSACTIONS.md`: one row per transaction, in broadcast order, with
the signer each one requires.

Addresses in the rendered list are FORK addresses. They are reproducible (they derive from the
governance nonce at the pinned fork block) but they are NOT the addresses a live run will produce,
because the live nonce will have moved. The list is authoritative about ORDER, SENDER, TARGET and
CALLDATA SHAPE - never about where a contract will land. That distinction is stated in the output so
nobody transcribes a fork address into a config.

Usage: scripts/render-cutover-txlist.py [broadcast-json] [output.md]
"""
import json
import os
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
DEFAULT_IN = os.path.join(
    ROOT, "contracts", "broadcast", "RehearseCutover.s.sol", "135", "run-latest.json"
)
DEFAULT_OUT = os.path.join(ROOT, "docs", "CUTOVER_TRANSACTIONS.md")

# Which C-step each transaction is, and who must sign it. Keyed by (contractName, function).
# A deployment needs NO authority; a call into an already-deployed contract needs governance.
STEPS = {
    ("ProviderRegistry", None): (
        "C-1",
        "any",
        "Deploy the provider authority core, empty.",
    ),
    ("DogTagIssuerV2", None): (
        "C-3a",
        "any",
        "Deploy the generation-2 issuer implementation.",
    ),
    ("CloneProvenanceRouter", None): (
        "C-4",
        "any",
        "Deploy the provenance router over [generation 1] ONLY, oldest first.",
    ),
    ("DogTagIssuerFactoryV2", None): (
        "C-3b",
        "any",
        "Deploy the generation-2 factory, bound to the core and the router.",
    ),
    ("CloneProvenanceRouter", "appendGeneration(address)"): (
        "C-4b",
        "GOVERNANCE",
        "Append generation 2 to the router, at the tail.",
    ),
    ("ProviderRegistry", "addFactoryGeneration(bytes32,address)"): (
        "C-2",
        "GOVERNANCE",
        "Record the generation-2 factory as a recognised generation.",
    ),
    ("VerificationRegistryConsent", None): (
        "C-5",
        "any",
        "Deploy the generation-2 verification registry over the router, reusing SBT and verifier.",
    ),
    ("ProtocolRegistryV2", None): (
        "C-8",
        "any",
        "Deploy the generation-2 discovery registry with a non-zero publish timelock.",
    ),
}


def main() -> int:
    src = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_IN
    dst = sys.argv[2] if len(sys.argv) > 2 else DEFAULT_OUT

    if not os.path.exists(src):
        sys.exit(
            "no broadcast at %s - run scripts/rehearse-cutover.sh first" % src
        )

    d = json.load(open(src))
    txs, receipts = d["transactions"], d["receipts"]
    if len(txs) != len(receipts):
        sys.exit("transaction/receipt count mismatch - refusing to render a partial list")

    rows, unknown = [], []
    for i, (t, r) in enumerate(zip(txs, receipts), start=1):
        name = t.get("contractName")
        fn = t.get("function")
        key = (name, fn)
        if key not in STEPS:
            unknown.append(key)
            step, signer, what = "?", "?", "UNMAPPED - this renderer does not know this step"
        else:
            step, signer, what = STEPS[key]
        rows.append(
            {
                "n": i,
                "step": step,
                "kind": t["transactionType"],
                "name": name,
                "fn": fn or "(constructor)",
                "signer": signer,
                "status": r["status"],
                "what": what,
            }
        )

    if unknown:
        # A new step that this renderer silently labelled "?" would read as a complete list.
        sys.exit(
            "unmapped transactions %s - add them to STEPS rather than shipping a list with holes"
            % unknown
        )

    # The mirror of the check above: that one refuses a step this renderer does not know, this one
    # refuses a step it knows and did not receive. Without it a broadcast that skipped a whole
    # C-step - C-4b being the one whose omission is fatal and silent - would render as a complete,
    # captain-approvable list. STEPS enumerates exactly the steps this sequence broadcasts, so the
    # comparison is available rather than something the renderer "cannot notice".
    expected_steps = {step for step, _, _ in STEPS.values()}
    missing = sorted(expected_steps - {r["step"] for r in rows})
    if missing:
        sys.exit(
            "the broadcast is missing step(s) %s - refusing to render a list that would read as the "
            "whole sequence" % missing
        )

    # A DOCUMENT THAT MISSTATES THE OUTCOME IS WORSE THAN A STALE ONE.
    #
    # The header below asserts every transaction succeeded, so it may only ever be written when that
    # is true. Rendering anyway and appending a contradicting failure line put a file on disk whose
    # opening claim was false - and this file exists for the captain to approve a live cutover from.
    # Refusing leaves the previously committed list in place, deliberately: stale-but-true beats
    # fresh-but-false, and the rehearsal wrapper fails on the same receipts before reaching here.
    failed = [r for r in rows if r["status"] != "0x1"]
    if failed:
        for r in failed:
            print(
                "  tx %d (%s) %s.%s -> status %s"
                % (r["n"], r["step"], r["name"], r["fn"], r["status"]),
                file=sys.stderr,
            )
        sys.exit(
            "%d of %d broadcast transactions did NOT succeed - refusing to write %s, which would "
            "claim they did. The previously committed list is left untouched." % (len(failed), len(rows), dst)
        )

    out = []
    out.append("<!-- GENERATED by scripts/render-cutover-txlist.py - do not hand-edit. -->")
    out.append("# Cutover transaction list (rehearsed)\n")
    out.append(
        "Captured from a rehearsal of the whole sequence against a fork of ROAX at block "
        "**%d**, chain **%s**. Every transaction below was executed against the REAL deployed "
        "bytecode and returned success.\n" % (
            json.load(
                open(os.path.join(ROOT, "contracts", "rehearsal", "fixtures", "historical-roots.json"))
            )["pinnedBlock"],
            d.get("chain"),
        )
    )
    out.append(
        "> **The addresses are FORK addresses.** They derive from the governance account's nonce at "
        "the pinned block and will differ on a live run. This list is authoritative about the "
        "ORDER, the SIGNER and the CALLDATA of each step - never about where a contract lands. Do "
        "not transcribe an address from here into any config.\n"
    )
    out.append("| # | step | signer | transaction | what it does |")
    out.append("|---|------|--------|-------------|--------------|")
    for r in rows:
        target = (
            "deploy `%s`" % r["name"]
            if r["kind"] == "CREATE"
            else "`%s.%s`" % (r["name"], r["fn"])
        )
        out.append(
            "| %d | **%s** | %s | %s | %s |"
            % (r["n"], r["step"], r["signer"], target, r["what"])
        )

    gov_steps = [r for r in rows if r["signer"] == "GOVERNANCE"]
    out.append("")
    out.append(
        "%d of the %d transactions require the governance key; the other %d are plain deployments "
        "that grant the deployer nothing.\n"
        % (len(gov_steps), len(rows), len(rows) - len(gov_steps))
    )
    # The renderer refuses an unmapped transaction AND a missing known step (both above), so the list
    # cannot silently be short. What it cannot infer is which steps were never IN this sequence, so
    # those exclusions are stated rather than left to a reader to notice.
    out.append("## Steps deliberately NOT in this list\n")
    out.append(
        "This list is the on-chain sequence the rehearsal could execute end to end. It is **not** "
        "the whole cutover. The steps below are excluded because each is one transaction per signer "
        "or per relayer, and the membership of those sets is KYC work (plan §4) rather than a chain "
        "fact - a list with invented addresses would read as ready.\n"
    )
    out.append("| step | why it is absent |")
    out.append("|------|------------------|")
    out.append(
        "| **C-2** (the rest) | Registering providers and mirroring issuance grants needs plan §4 "
        "items 1-6. Attaching the five generation-1 clones is not possible at all - they have no "
        "`owner()`. Only the factory-generation record is in this list. |"
    )
    out.append(
        "| **C-6** | One tx per `(purpose, relayer)`; which relayer serves which purpose is plan §4 "
        "item 5. Roughly 7 of the 33 live grants sit on this axis. **Assertion 5 is this step being "
        "applied and withheld on one credential**, so the mechanics are proven. |"
    )
    out.append("| **C-7** | `ServiceDomainResolver` and `ProviderDirectory` have slack and gate nothing else. |")
    out.append("| **C-9, C-10** | Client repointing and an app release - no transactions. |")
    out.append(
        "| **C-10b, C-11** | One tx per signer, from the same reconciliation. Mechanics exercised on "
        "the fork - C-10b is the SBT `grantRole` in the fork test's `setUp`, C-11 is "
        "`setIssuanceCapability` in the generation-2 anchoring helper. Not enumerated here. |"
    )
    out.append(
        "| **C-12** | One tx per `(recordType, signer)`. **Assertion 7 performs a real `delistFor`** "
        "and shows new generation-1 issuance refused while all historical roots still verify - the "
        "property that makes the freeze safe. Not enumerated here. |"
    )
    out.append("")

    out.append("Regenerate with `make rehearse-cutover`, which runs this renderer as its last step.")
    out.append("")

    os.makedirs(os.path.dirname(dst), exist_ok=True)
    open(dst, "w").write("\n".join(out))
    print("wrote %d transactions to %s" % (len(rows), dst))
    return 0


if __name__ == "__main__":
    sys.exit(main())

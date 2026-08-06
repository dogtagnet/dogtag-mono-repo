#!/usr/bin/env python3
"""The mutations `verify-deployment-record-mutations.sh` applies, one per claim.

Kept in Python rather than inline in the shell script because the scrutinees are TypeScript source
containing `$`, backticks and backslashes - every one of which bash re-interprets inside a heredoc.
Two mutations were silently reported INERT for exactly that reason on the first run, and an inert
mutation reads like an unpinned claim, which is the one thing this harness exists not to do.

Each entry: (file, label, suite, expect-red, old, new). `old` must be present or the mutation is
inert and the harness says so rather than counting the resulting green as evidence.
"""

import pathlib
import sys

DEPLOY = "packages/ui/src/provider/deploymentHistory.ts"
DOMAIN = "packages/ui/src/provider/domainClaim.ts"
DIRECTORY = "packages/ui/src/provider/directoryPlan.ts"
CLONE = "packages/ui/src/provider/cloneProvenance.ts"
AVAIL = "packages/ui/src/provider/actionAvailability.ts"
FLOWS = "packages/ui/src/domain/ProviderSelfServiceFlows.tsx"
SEND = "packages/ui/src/provider/sendOutcome.ts"

RECORD_SUITE = "test/providerDeploymentRecord.test.ts"
STRANDED_SUITE = "test/providerStrandedStates.test.ts"
SEND_SUITE = "test/providerSendOutcome.test.ts"
SPLIT_SUITE = "test/providerCapabilitySplit.test.tsx"

MUTATIONS = [
    # ---- the record read back from the chain -----------------------------------------------
    (
        DEPLOY,
        "filter the creation log by provider id as well as owner",
        RECORD_SUITE,
        "lists a contract deployed under a DIFFERENT provider id",
        "    logs = await reader.issuerCreations(owner);",
        '    logs = (await reader.issuerCreations(owner)).filter(\n'
        '      (l) => l.providerId === "0x7b160cb6dd3f8690247093c16d16a21e61d98eea",\n'
        "    );",
    ),
    (
        DEPLOY,
        "render a failed log read as an empty list",
        RECORD_SUITE,
        "reports a failed log read as unread",
        '    return {\n      state: "couldNotRead",\n'
        '      reason: reasonFrom(error, "the factory\'s creation log could not be read"),\n    };',
        '    return { state: "read", deployments: [] };',
    ),
    (
        DEPLOY,
        "drop a log that carries no block position",
        RECORD_SUITE,
        "keeps a log with no block position",
        "  const deployments = await Promise.all(\n    logs.map",
        "  const deployments = await Promise.all(\n    logs.filter((l) => l.blockNumber !== undefined).map",
    ),
    (
        DEPLOY,
        "withhold a whole row when its record-type read fails",
        RECORD_SUITE,
        "keeps a contract listed when its own follow-up reads fail",
        '    return { recordTypeReason: reasonFrom(error, "the recordType() read failed") };',
        "    throw error;",
    ),
    # ---- the next contract number ------------------------------------------------------------
    (
        DEPLOY,
        "fold the highest number across every record type",
        RECORD_SUITE,
        "is scoped BY RECORD TYPE",
        "    .filter((d) => d.recordType === recordType)",
        "    .filter(() => true)",
    ),
    (
        DEPLOY,
        "guess a number when a record type could not be resolved",
        RECORD_SUITE,
        "refuses to guess when ANY contract",
        "  const unresolved = history.deployments.find((d) => d.recordType === undefined);",
        "  const unresolved = undefined as undefined | (typeof history.deployments)[number];",
    ),
    (
        DEPLOY,
        "let a non-numeric contract number through to BigInt",
        RECORD_SUITE,
        "refuses what BigInt would have thrown on",
        "  if (!/^\\d+$/.test(trimmed)) {",
        "  if (false as boolean) {",
    ),
    (
        DEPLOY,
        "accept a contract number the uint96 argument cannot carry",
        RECORD_SUITE,
        "refuses a number the uint96 argument cannot carry",
        "  if (value > MAX_CLONE_NONCE) {",
        "  if (false as boolean) {",
    ),
    # ---- the states that stranded him --------------------------------------------------------
    (
        CLONE,
        "call a pending attachment frozen again",
        STRANDED_SUITE,
        "is NOT reported as frozen",
        "      awaitingRegistrar =\n"
        "        effective.providerStanding === Standing.PENDING\n"
        "        || effective.serviceStanding === Standing.PENDING;",
        "      awaitingRegistrar = false as boolean;",
    ),
    (
        CLONE,
        "blame the key for a refusal that is about standing",
        STRANDED_SUITE,
        "does not blame the operator's key",
        "      } else if (standingEffective === true) {",
        "      } else if (true as boolean) {",
    ),
    (
        CLONE,
        "stop blaming the key even when standing is fine",
        STRANDED_SUITE,
        "DOES blame the key when standing is established as fine",
        "      } else if (standingEffective === true) {",
        "      } else if (false as boolean) {",
    ),
    (
        CLONE,
        "restore the claim that no page exists for attaching",
        STRANDED_SUITE,
        "does not claim there is no page for attaching a contract",
        "export const ATTACHMENT_IS_A_DOGTAG_STEP =\n  \"Before you can select",
        "export const ATTACHMENT_IS_A_DOGTAG_STEP =\n"
        '  "DogTag has to attach it to your provider record, and there is no page for it yet." + "" + "Before you can select',
    ),
    (
        CLONE,
        "send the summary line back to 'the failed checks above say why'",
        STRANDED_SUITE,
        "says WAIT in the one-line summary",
        '      if (awaitingRegistrar) {\n        return "Nothing for you to do here yet:',
        '      if (false as boolean) {\n        return "Nothing for you to do here yet:',
    ),
    (
        DOMAIN,
        "blame the key when the domain register is not live",
        STRANDED_SUITE,
        "flow 3 does not blame the key",
        "    } else if (resolverLive === true) {",
        "    } else if (true as boolean) {",
    ),
    (
        DOMAIN,
        "stop blaming the key even when the register IS live",
        STRANDED_SUITE,
        "flow 3 DOES blame the key",
        "    } else if (resolverLive === true) {",
        "    } else if (false as boolean) {",
    ),
    (
        DIRECTORY,
        "blame the key when the provider record is not yet active",
        STRANDED_SUITE,
        "flow 4 names a PENDING provider record",
        "    } else if (providerActive === true) {",
        "    } else if (true as boolean) {",
    ),
    (
        FLOWS,
        "spend a reason on a flow the groomer does not render",
        SPLIT_SUITE,
        "still explains its blocked control IN FULL",
        "    ...(capabilities.issuance\n      ? ([\n          deployCheckBlock,",
        "    ...(true as boolean\n      ? ([\n          deployCheckBlock,",
    ),
    (
        AVAIL,
        "print a repeated reason verbatim again",
        STRANDED_SUITE,
        "says a repeated obstacle briefly",
        '    if (seen.has(text)) return { block, style: "brief" };',
        '    if (seen.has(text)) return { block, style: "full" };',
    ),
    (
        AVAIL,
        "suppress a repeated reason entirely, silencing the control",
        STRANDED_SUITE,
        "never leaves a control silent",
        '    if (seen.has(text)) return { block, style: "brief" };',
        "    if (seen.has(text)) return null;",
    ),
    (
        AVAIL,
        "dedupe on the block KIND rather than the sentence",
        STRANDED_SUITE,
        "compares the SENTENCE, not the kind",
        "    const text = describeActionBlock(block);",
        "    const text = block.kind;",
    ),
    (
        AVAIL,
        "instruct a Check that is unavailable",
        STRANDED_SUITE,
        "does not tell you to check again when Check is unavailable",
        "  return checkBlocked\n    ?",
        "  return (false as boolean)\n    ?",
    ),
    (
        AVAIL,
        "restate the whole obstacle in the banner instead of naming it briefly",
        STRANDED_SUITE,
        "names the obstacle briefly",
        "briefActionBlock(\n        checkBlocked,\n      )",
        "describeActionBlock(\n        checkBlocked,\n      ) + describeActionBlock({ kind: \"notConnected\" })",
    ),
    # ---- the address carried on a send row ---------------------------------------------------
    (
        SEND,
        "present a reverted deploy's address as a contract that exists",
        SEND_SUITE,
        "never says a contract exists at an address nothing was created at",
        '      return "Nothing was created. This is the address the attempt would have created:";',
        '      return "This contract now exists at:";',
    ),
    (
        SEND,
        "present a pending deploy's address as a contract that exists",
        SEND_SUITE,
        "never says a contract exists at an address nothing was created at",
        '      return "The address this will create, if it succeeds:";',
        '      return "This contract now exists at:";',
    ),
    (
        SEND,
        "drop the address from the row entirely",
        SEND_SUITE,
        "carries the predicted address",
        "  const created = createdAddress ? { createdAddress } : {};",
        "  const created = {};",
    ),
]

# The two deliberate self-tests: the harness must report each as INERT rather than as evidence.
SELF_TESTS = [
    (
        DEPLOY,
        "SELF-TEST a mutation whose scrutinee does not exist",
        RECORD_SUITE,
        "never printed",
        "a string that appears nowhere in this file at all",
        "x",
    ),
    (
        DEPLOY,
        "SELF-TEST a mutation that does not compile",
        RECORD_SUITE,
        "never printed",
        "export const MAX_CLONE_NONCE",
        "export const MAX_CLONE_NONCE: = (((",
    ),
]

ALL = MUTATIONS + SELF_TESTS


def main() -> int:
    if sys.argv[1] == "count":
        print(len(ALL))
        return 0
    if sys.argv[1] == "describe":
        f, label, suite, expect, _, _ = ALL[int(sys.argv[2])]
        print(f"{f}\t{label}\t{suite}\t{expect}")
        return 0
    if sys.argv[1] == "apply":
        f, _, _, _, old, new = ALL[int(sys.argv[2])]
        p = pathlib.Path(f)
        s = p.read_text()
        if old not in s:
            return 1
        p.write_text(s.replace(old, new, 1))
        return 0
    raise SystemExit(f"unknown command {sys.argv[1]}")


if __name__ == "__main__":
    raise SystemExit(main())

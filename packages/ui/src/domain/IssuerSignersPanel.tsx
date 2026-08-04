/**
 * Who may sign in this provider's name — the product surface for LAYER 2 of the two-layer issuance
 * requirement.
 *
 * # The gap this closes
 *
 * Issuing needs two permissions. The authority core grants the right, and that has a screen (the
 * admin registrar surface). The provider's OWN contract must then accept that signer
 * (`issuanceAllowed`), and that had no screen, no button and no backend route anywhere — so a real
 * provider is approved, deploys their contract, and still cannot issue, because their practice
 * software signs with the backend's custody key rather than with whoever clicked Deploy. The crew
 * that walked `docs/DEMO_CLICKS.md` had to send that transaction from a terminal to get past its
 * own step 3.
 *
 * # Why the WRITE is a wallet transaction and not a backend call
 *
 * `setIssuanceAllowed` admits only from the contract's `owner()`, and the protocol admin is
 * deliberately excluded from that direction: it also writes the authority bit, so a registrar that
 * could admit would hold both layers at once and reach exactly the cross-provider issuance layer 2
 * exists to prevent. The backend is not the owner either — its custody signer is the address that
 * needs ADMITTING — and it cannot authenticate one, because an operator session proves "staff of
 * this shop", never "owner of this contract". So the owner signs, from their own wallet, and the
 * backend only READS.
 *
 * # What this page may and may not claim
 *
 * * A list that could not be read renders as an unreadable list, never as an empty one.
 * * A withdrawn signer carries the WORD "Withdrawn", so the distinction survives a screen reader, a
 *   screenshot and a flattened text dump — the failure mode the guide-walk crew hit on a
 *   neighbouring screen, where it was carried by styling alone.
 * * A submitted transaction is never rendered as a completed grant. The roster is re-read only
 *   after a receipt reports success.
 */

import { useCallback, useEffect, useState, type ReactNode } from "react";
import { useAccount, useWriteContract } from "wagmi";
import { Badge } from "../components/Badge";
import { Button } from "../components/Button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "../components/Card";
import { Input } from "../components/Input";
import { Label } from "../components/Label";
import { CopyButton } from "../chain/ChainValue";
import {
  admitBlock,
  backendSignerTone,
  backendSignerVerdict,
  describeActionBlock,
  describeBackendSignerVerdict,
  normalizeAddress,
  removeBlock,
  signerStanding,
  signerStandingDetail,
  signerStandingLabel,
  signerStandingTone,
  type IssuanceAllowedResponse,
  type IssuerContract,
  type RosterEntry,
} from "../signers";
import { validateSignerInput } from "../signers/roster";
import {
  mayContinueAfter,
  outcomeFromReceiptStatus,
  sendExplorerHref,
  sendRecord,
  sendStateLabel,
  type SendRecord,
  type SendState,
} from "../provider/sendOutcome";
import { ROAX_CHAIN_ID } from "../wallet/chain";
import { classifySurfaceFault, type SurfaceFault } from "../wallet/walletError";
import { roaxPublicClient } from "../wallet/contracts";
import { WalletFaultNotice } from "./ProviderSelfServicePanel";

/** The one write this page makes. Four bytes derived by viem from this signature, never a literal. */
const ISSUANCE_ALLOWED_ABI = [
  {
    type: "function",
    name: "setIssuanceAllowed",
    stateMutability: "nonpayable",
    inputs: [
      { name: "signer", type: "address" },
      { name: "allowed", type: "bool" },
    ],
    outputs: [],
  },
] as const;

const WALLET_TIMEOUT_MS = 90_000;
const RECEIPT_TIMEOUT_MS = 120_000;

export interface IssuerSignersPanelProps {
  /** Reads `GET /issuer/issuance-allowed` on this deployment's backend. */
  load: () => Promise<IssuanceAllowedResponse>;
  /** The endpoint receipts are followed through. The wallet supplies its own for the send. */
  rpcUrl: string;
}

type Load =
  | { state: "loading" }
  | { state: "loaded"; data: IssuanceAllowedResponse }
  | { state: "failed"; reason: string };

export function IssuerSignersPanel({ load, rpcUrl }: IssuerSignersPanelProps): ReactNode {
  const { address, isConnected, chainId } = useAccount();
  const { writeContractAsync } = useWriteContract();

  // THREE states, and `loading` is its own. A two-value `data | null` makes the INITIAL value the
  // failure value, so the page announces "the list could not be read" before it has asked — the
  // same collapse the admin Providers banner had to fix.
  const [load_, setLoad] = useState<Load>({ state: "loading" });
  const [busy, setBusy] = useState(false);
  const [fault, setFault] = useState<SurfaceFault | null>(null);
  const [sent, setSent] = useState<SendRecord[]>([]);
  const [signerInput, setSignerInput] = useState<Record<string, string>>({});

  const refresh = useCallback(async () => {
    setLoad({ state: "loading" });
    try {
      setLoad({ state: "loaded", data: await load() });
    } catch (e) {
      setLoad({ state: "failed", reason: e instanceof Error ? e.message : String(e) });
    }
  }, [load]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  /**
   * Send one `setIssuanceAllowed` and report what actually happened to it.
   *
   * Ported from `ProviderSelfServiceFlows.sendAndFollow` rather than re-invented, including the two
   * states that exist because a captain's transaction mined and the page showed nothing: the row is
   * recorded BEFORE the wallet is asked, and a wallet that never answers is `walletSilent` rather
   * than silence.
   */
  const sendAndFollow = useCallback(
    async (what: string, send: () => Promise<`0x${string}`>): Promise<SendState> => {
      const id = `send-${Date.now()}-${Math.round(Math.random() * 1e6)}`;
      const settle = (r: SendRecord) =>
        setSent((s) => s.map((x) => (x.id === r.id ? r : x)));
      setSent((s) => [sendRecord(id, what, "awaitingWallet"), ...s]);

      const follow = async (hash: `0x${string}`): Promise<SendState> => {
        try {
          const receipt = await roaxPublicClient(rpcUrl).waitForTransactionReceipt({
            hash,
            timeout: RECEIPT_TIMEOUT_MS,
          });
          const state = outcomeFromReceiptStatus(receipt.status);
          settle(sendRecord(id, what, state, { hash }));
          return state;
        } catch (e) {
          // Neither success nor failure: a receipt we could not fetch says nothing about whether the
          // transaction mined.
          const reason = e instanceof Error ? e.message.split("\n")[0]! : String(e);
          settle(sendRecord(id, what, "unknown", { hash, unknownReason: reason }));
          return "unknown";
        }
      };

      const pending = send();
      let timedOut = false;
      let hash: `0x${string}`;
      try {
        hash = await Promise.race([
          pending,
          new Promise<never>((_, reject) =>
            setTimeout(() => {
              timedOut = true;
              reject(new Error("wallet-timeout"));
            }, WALLET_TIMEOUT_MS),
          ),
        ]);
      } catch (e) {
        if (!timedOut) {
          // A genuine rejection: nothing was submitted, so the attempt is withdrawn rather than left
          // on screen as something that might have happened.
          setSent((s) => s.filter((r) => r.id !== id));
          throw e;
        }
        settle(
          sendRecord(id, what, "walletSilent", {
            unknownReason:
              "Your wallet did not respond in time. It may still have sent this — check your wallet's "
              + "activity, then reload this page: the list is read from the chain, so it will show the "
              + "change if it landed.",
          }),
        );
        void pending.then((late) => void follow(late)).catch(() => {});
        return "walletSilent";
      }

      settle(sendRecord(id, what, "submitted", { hash }));
      return follow(hash);
    },
    [rpcUrl],
  );

  const write = useCallback(
    async (issuerAddr: string, signer: string, allowed: boolean) => {
      setBusy(true);
      setFault(null);
      try {
        const what = `${allowed ? "Admit" : "Remove"} ${signer} on ${issuerAddr}`;
        const state = await sendAndFollow(what, () =>
          writeContractAsync({
            address: issuerAddr as `0x${string}`,
            abi: ISSUANCE_ALLOWED_ABI,
            functionName: "setIssuanceAllowed",
            args: [signer as `0x${string}`, allowed],
          }),
        );
        // ONLY an established success re-reads. A submitted-but-unsettled transaction must never
        // flip a row to "Can issue" — that is the whole "never show a pending transaction as a
        // completed grant" requirement, and re-reading on anything weaker would do exactly that on
        // a chain that had not applied it yet.
        if (mayContinueAfter(state)) {
          if (allowed) setSignerInput((s) => ({ ...s, [issuerAddr]: "" }));
          await refresh();
        }
      } catch (e) {
        setFault(classifySurfaceFault(e));
      } finally {
        setBusy(false);
      }
    },
    [refresh, sendAndFollow, writeContractAsync],
  );

  if (load_.state === "loading") {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Who may sign in your name</CardTitle>
          <CardDescription>Reading your contracts&rsquo; issuance lists from the chain…</CardDescription>
        </CardHeader>
      </Card>
    );
  }

  if (load_.state === "failed") {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Who may sign in your name</CardTitle>
          <CardDescription data-testid="signers-load-failed">
            This page could not reach your backend, so nothing about your contracts has been
            established. This is not a statement that nobody may issue.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted">{load_.reason}</p>
          <Button className="mt-3" onClick={() => void refresh()}>
            Try again
          </Button>
        </CardContent>
      </Card>
    );
  }

  const { activeSigner, contracts } = load_.data;

  return (
    <div className="space-y-4">
      <Card>
        <CardHeader>
          <CardTitle>Who may sign in your name</CardTitle>
          <CardDescription>
            Issuing takes two separate permissions, and this page is the second one. DogTag grants
            your practice the right to issue; your own contract then decides which keys may use it.
            Both must hold, so a key missing from the list below cannot issue however DogTag has set
            the first.
          </CardDescription>
        </CardHeader>
        <CardContent>
          <p className="text-sm text-muted">
            You sign these changes yourself, from the wallet that owns the contract. There is no
            button here for DogTag to do it for you, and that is deliberate: DogTag also grants the
            first permission, so a DogTag that could add keys to your contract would hold both halves
            at once.
          </p>
          {activeSigner ? (
            <p className="mt-2 text-sm">
              This shop signs with{" "}
              <span className="font-mono text-xs" data-testid="active-signer">
                {activeSigner}
              </span>
              <CopyButton value={activeSigner} label="signing key" className="ml-1" />
            </p>
          ) : (
            <p className="mt-2 text-sm" data-testid="no-active-signer">
              Custody is locked, so this shop&rsquo;s signing key cannot be derived. Unlock it and
              reload to see whether it may issue.
            </p>
          )}
        </CardContent>
      </Card>

      {contracts.length === 0 ? (
        <Card>
          <CardContent className="pt-6">
            <p className="text-sm" data-testid="no-contracts">
              No issuing contract is configured on this deployment, so there is no issuance list to
              show.
            </p>
          </CardContent>
        </Card>
      ) : null}

      {contracts.map((c) => (
        <ContractCard
          key={c.issuerAddr}
          contract={c}
          activeSigner={activeSigner}
          account={address ? normalizeAddress(address) : null}
          connected={isConnected && !!address}
          chainId={chainId}
          busy={busy}
          input={signerInput[c.issuerAddr] ?? ""}
          onInput={(v) => setSignerInput((s) => ({ ...s, [c.issuerAddr]: v }))}
          onWrite={write}
        />
      ))}

      {/* SHARED with the S-15 flows rather than re-rendered here. A wallet answering `4100
          Unauthorized` must not read as a statement about this provider's authority, and the rule
          that stops it - label the layer, deny what was not established, put the wallet's own words
          last - belongs in one place. */}
      <WalletFaultNotice fault={fault} />

      {sent.length ? (
        <Card>
          <CardHeader>
            <CardTitle>Transactions this page has sent</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            {sent.map((r) => {
              const href = sendExplorerHref(r);
              return (
                <div key={r.id} className="text-sm" data-testid="send-record">
                  <span>{r.what}</span>
                  {" — "}
                  <span data-testid="send-state">{sendStateLabel(r.state)}</span>
                  {href ? (
                    <>
                      {" "}
                      <a className="underline" href={href} target="_blank" rel="noreferrer">
                        explorer
                      </a>
                    </>
                  ) : null}
                  {r.unknownReason ? (
                    <p className="text-xs text-muted">{r.unknownReason}</p>
                  ) : null}
                </div>
              );
            })}
          </CardContent>
        </Card>
      ) : null}
    </div>
  );
}

function ContractCard({
  contract,
  activeSigner,
  account,
  connected,
  chainId,
  busy,
  input,
  onInput,
  onWrite,
}: {
  contract: IssuerContract;
  activeSigner: string | null;
  account: string | null;
  connected: boolean;
  chainId: number | undefined;
  busy: boolean;
  input: string;
  onInput: (v: string) => void;
  onWrite: (issuerAddr: string, signer: string, allowed: boolean) => void | Promise<void>;
}): ReactNode {
  const { read, issuerAddr, recordType } = contract;
  const verdict = backendSignerVerdict(activeSigner, read);
  const tone = backendSignerTone(verdict);
  const wallet = { busy, connected, account, expectedChainId: ROAX_CHAIN_ID, actualChainId: chainId };
  const inputProblem = validateSignerInput(input, read);
  const admit = admitBlock({ ...wallet, read, inputProblem });

  return (
    // Scoped, so a reader - and a test - can speak about ONE contract's list. Without it every
    // address that appears on two contracts (the owner, and this shop's own signer, which always
    // gets a row) is ambiguous across the page.
    <Card data-testid="issuer-contract" data-record-type={recordType}>
      <CardHeader>
        <CardTitle>
          {recordType}{" "}
          <span className="font-mono text-xs font-normal text-muted">{issuerAddr}</span>
        </CardTitle>
        <CardDescription
          data-testid="backend-signer-verdict"
          className={tone === "bad" ? "text-danger" : tone === "warn" ? "text-warning" : undefined}
        >
          {describeBackendSignerVerdict(verdict)}
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {read.state === "unavailable" ? (
          // NEITHER NEIGHBOUR. Not an empty list, and not a refusal. The one thing this arm may
          // never do is render as "nobody is admitted".
          <div
            className="rounded border border-warning/40 bg-warning/5 p-3 text-sm"
            data-testid="roster-unavailable"
          >
            <p className="font-medium">This contract&rsquo;s list could not be read.</p>
            <p className="mt-1 text-muted">
              That is not the same as nobody being admitted — it means the question was not answered.
              Nothing below is a statement about who may issue.
            </p>
            <p className="mt-1 font-mono text-xs">{read.reason}</p>
          </div>
        ) : (
          <>
            <p className="text-sm text-muted">
              Owned by{" "}
              <span className="font-mono text-xs" data-testid="clone-owner">
                {read.owner}
              </span>
              <CopyButton value={read.owner} label="owner address" className="ml-1" />
            </p>

            {read.entries.length === 0 ? (
              <p className="text-sm" data-testid="roster-empty">
                This contract&rsquo;s issuance list is empty. It was read successfully — no key may
                sign through it.
              </p>
            ) : (
              <ul className="space-y-2" data-testid="roster">
                {read.entries.map((e) => (
                  <SignerRow
                    key={e.address}
                    entry={e}
                    isOurs={!!activeSigner && normalizeAddress(activeSigner) === e.address}
                    block={removeBlock({ ...wallet, read, entry: e })}
                    onRemove={() => void onWrite(issuerAddr, e.address, false)}
                  />
                ))}
              </ul>
            )}

            <div className="border-t pt-3">
              <Label htmlFor={`admit-${issuerAddr}`}>Add a signing key</Label>
              <div className="mt-1 flex gap-2">
                <Input
                  id={`admit-${issuerAddr}`}
                  data-testid="admit-input"
                  placeholder="0x…"
                  value={input}
                  onChange={(ev) => onInput(ev.target.value)}
                />
                <Button
                  data-testid="admit-submit"
                  disabled={!!admit}
                  onClick={() => void onWrite(issuerAddr, normalizeAddress(input), true)}
                >
                  Admit
                </Button>
              </div>
              {admit ? (
                <p className="mt-2 text-xs text-muted" data-testid="admit-blocked">
                  {describeActionBlock(admit)}
                </p>
              ) : null}
              {activeSigner
              && read.state === "resolved"
              && read.activeSignerAllowed === false
              && normalizeAddress(input) !== normalizeAddress(activeSigner) ? (
                <button
                  type="button"
                  className="mt-2 text-xs underline"
                  data-testid="fill-our-signer"
                  onClick={() => onInput(activeSigner)}
                >
                  Use this shop&rsquo;s signing key
                </button>
              ) : null}
            </div>
          </>
        )}
      </CardContent>
    </Card>
  );
}

function SignerRow({
  entry,
  isOurs,
  block,
  onRemove,
}: {
  entry: RosterEntry;
  isOurs: boolean;
  block: ReturnType<typeof removeBlock>;
  onRemove: () => void;
}): ReactNode {
  const standing = signerStanding(entry);
  return (
    <li className="rounded border p-2 text-sm" data-testid="roster-row">
      <div className="flex flex-wrap items-center gap-2">
        <span className="font-mono text-xs" data-testid="roster-address">
          {entry.address}
        </span>
        <CopyButton value={entry.address} label="signer address" />
        {/* The WORD is the carrier. `Badge` styles it; it never replaces it, so the distinction
            survives a screen reader and a flattened text dump of this page. */}
        {/* The variant comes from `signerStandingTone`, not an inline ternary: a second copy of the
            rule here would leave that function's tests pinning nothing that ships. Tone is applied
            ON TOP of the word, never instead of it. */}
        <Badge
          data-testid="roster-standing"
          variant={signerStandingTone(standing) === "ok" ? "success" : "neutral"}
          className={standing === "withdrawn" ? "line-through" : undefined}
        >
          {signerStandingLabel(standing)}
        </Badge>
        {isOurs ? <Badge variant="outline">this shop&rsquo;s signing key</Badge> : null}
        <span className="ml-auto">
          <Button
            size="sm"
            variant="outline"
            data-testid="remove-signer"
            disabled={!!block}
            onClick={onRemove}
          >
            Remove
          </Button>
        </span>
      </div>
      <p className="mt-1 text-xs text-muted" data-testid="roster-detail">
        {signerStandingDetail(standing)}
      </p>
      {block ? (
        <p className="mt-1 text-xs text-muted" data-testid="remove-blocked">
          {describeActionBlock(block)}
        </p>
      ) : null}
    </li>
  );
}

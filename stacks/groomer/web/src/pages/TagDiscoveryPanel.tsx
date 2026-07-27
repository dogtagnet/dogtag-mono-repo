import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  ChainTime,
  ChainValue,
  Spinner,
  addressExplorerHref,
  attributeSigner,
  discoverTag,
  resolveDogTagId,
  txExplorerHref,
  useToast,
  type DiscoveredEvent,
  type DiscoveryProgress,
  type TagDiscoveryResult,
  type TagStatusLabel,
} from "@dogtag/ui";
import { Radar, Search, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";

/**
 * ON-CHAIN DISCOVERY for one pet's DogTag: what the chain holds against this tag, including what this
 * shop has never seen.
 *
 * This is the surface that makes a returning owner workable. The shop's database knows only its own
 * history, so for a pet it has never groomed the tag is the only handle anyone holds. This panel reads
 * the chain directly — permissionless `eth_call`/`eth_getLogs` over the public RPC, the same
 * direct-to-RPC posture the credential verify panel uses — and reports what is genuinely recorded.
 *
 * Three properties are deliberate and load-bearing:
 *
 *  1. **Every row is a real log.** Nothing is seeded, inferred or placeholder. An empty result means
 *     the chain holds nothing in the window scanned, and says exactly that.
 *  2. **Coverage is stated, always.** A scan that could not finish is never rendered as one that
 *     finished and found nothing — the block range actually covered is shown, and any failed range is
 *     named. Swallowing a failed chunk here would be the same defect as rendering an unchecked
 *     credential as valid.
 *  3. **What is NOT discoverable is said out loud.** Level-B credential roots are not bound to a tag
 *     id on chain, by design, so no scan can enumerate "all this pet's vaccination records". What the
 *     chain does expose is the tag's own Level-A credential and every verification against it — and a
 *     verification by a relayer that is not this shop is positive evidence of credentials the shop has
 *     never seen. That is the cue to ask the owner to share them, which is where import comes in.
 *
 * `ProvenanceBadge` from the shared chain helpers is deliberately NOT used on these rows. It reports
 * whether a hash is *chain-ADDRESSABLE* for a surface that did not read the chain; here every row came
 * out of a log the RPC returned, so the badge's careful "this page does not read the chain" caveat
 * would be false in the other direction. The reads are anchored once, at panel level, instead.
 */
export function TagDiscoveryPanel({
  dogTagId,
  petId,
  petName,
}: {
  dogTagId: string;
  petId: string;
  petName: string;
}) {
  const { api, signerAddress } = useApp();
  const { toast } = useToast();
  const navigate = useNavigate();

  const [result, setResult] = useState<TagDiscoveryResult | null>(null);
  const [progress, setProgress] = useState<DiscoveryProgress | null>(null);
  const [scanning, setScanning] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  /**
   * The shop's own signer addresses, so "was this us?" is a fact rather than a guess - and `null`
   * when that set could NOT be established, which is a third state and not an empty one.
   *
   * `GET /issuer/signers` answers `200 {"signers": []}` whenever custody is locked, a state a groomer
   * reaches routinely. Collapsing that into an empty list would make every one of the shop's own
   * verifications render as a stranger's, and would drive the conclusion sentence and the import CTA
   * below off data the portal never had. Not knowing is reported as not knowing.
   */
  const [ownSigners, setOwnSigners] = useState<string[] | null>(null);

  const resolved = resolveDogTagId(dogTagId);

  useEffect(() => {
    let cancelled = false;
    void api
      .issuerSigners()
      .then((r) => {
        if (cancelled) return;
        // The whitelist matrix carries one row per (recordType, signer) pair, so the same address
        // appears repeatedly; only the distinct addresses matter here. Locked custody supplies
        // neither field, so this resolves to nothing - which stays `null`, not `[]`.
        const backend = [r.activeSigner, ...(r.matrix ?? []).map((m) => m.address)].filter(
          (a): a is string => Boolean(a && a.trim()),
        );
        const addrs = [...new Set(signerAddress ? [...backend, signerAddress] : backend)];
        setOwnSigners(addrs.length > 0 ? addrs : null);
      })
      .catch(() => {
        if (!cancelled) setOwnSigners(signerAddress ? [signerAddress] : null);
      });
    // Why `signerAddress` may make the set KNOWN on its own, rather than being a guess that reopens
    // the defect above: it is not a browser wallet. Every writer of it (`Setup`, `Unlock`, the
    // unlock dialog) sets it from `accounts[0]` of a custody unlock - custody account 0, which is
    // `ACTIVE_SIGNER_INDEX`, the address the verify path actually broadcasts from and the same one
    // `GET /issuer/signers` reports as `activeSigner` (its whole matrix is built from that one
    // address). So it is a real shop signer, cached from a previous unlock, and attributing against
    // it is a fact rather than an assumption.
    return () => {
      cancelled = true;
    };
  }, [api, signerAddress]);

  // Abort an in-flight scan if the operator navigates away mid-scan.
  useEffect(() => () => abortRef.current?.abort(), []);

  const scan = useCallback(async () => {
    if (!resolved) return;
    const ctrl = new AbortController();
    abortRef.current?.abort();
    abortRef.current = ctrl;
    setScanning(true);
    setError(null);
    setResult(null);
    setProgress(null);
    try {
      const r = await discoverTag({
        dogTagId: resolved.onchain,
        rpcUrl: env.roaxRpc,
        signal: ctrl.signal,
        onProgress: setProgress,
      });
      setResult(r);
    } catch (e) {
      // A failed point-in-time read has no result to qualify, so it is an error rather than an empty
      // scan — reporting "nothing found" here would be a fabrication.
      setError((e as Error).message);
      toast({
        title: "Could not read the chain",
        description: (e as Error).message,
        variant: "danger",
      });
    } finally {
      setScanning(false);
    }
  }, [resolved, toast]);

  const verifications = (result?.events ?? []).filter(
    (e): e is Extract<DiscoveredEvent, { kind: "verification" }> => e.kind === "verification",
  );
  // Only ever counted against a KNOWN signer set. With the set unknown these are not foreign
  // verifications, they are unattributed ones, and counting them would manufacture the finding.
  const foreignVerifications = verifications.filter(
    (v) => attributeSigner(v.relayer, ownSigners) === "other",
  );

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <Radar className="h-4 w-4 text-primary" /> On-chain discovery
        </CardTitle>
        <CardDescription>
          Read the chain for everything recorded against this pet's DogTag — including records this
          shop never created. Nothing here is written; every row is a log read from{" "}
          <span className="font-mono text-[11px]">{env.roaxRpc}</span>.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {!resolved ? (
          <p className="text-sm text-muted">
            {dogTagId
              ? `"${dogTagId}" is not a usable DogTag id, so there is nothing to scan for. A tag id is
                 either the short number the owner's app shows, or the full on-chain id.`
              : "Link a DogTag to this pet first — discovery needs a tag id to scan for."}
          </p>
        ) : (
          <>
            {/* What is about to be read, and under which id. The stored handle and the on-chain id are
                different values, and a scan on the wrong one would look like an empty chain. */}
            <div className="grid gap-2 rounded-md border border-border p-3 sm:grid-cols-2">
              {resolved.handle && (
                <ChainValue label="tag handle (as recorded here)" value={resolved.handle} stacked />
              )}
              <ChainValue
                label={
                  resolved.form === "handle"
                    ? "on-chain dog tag id (field hash of the handle)"
                    : "on-chain dog tag id"
                }
                value={resolved.onchain.toString()}
                stacked
              />
            </div>

            <div className="flex flex-wrap items-center gap-2">
              <Button size="sm" onClick={() => void scan()} disabled={scanning}>
                <Search className="h-4 w-4" /> {result ? "Scan again" : "Scan the chain"}
              </Button>
              {scanning && (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => abortRef.current?.abort()}
                >
                  <X className="h-4 w-4" /> Cancel
                </Button>
              )}
              {scanning && (
                <span className="inline-flex items-center gap-2 text-sm text-muted">
                  <Spinner className="h-4 w-4" />
                  {progress
                    ? `Scanning blocks ${progress.reachedBlock.toString()}–${progress.toBlock.toString()} · ${progress.chunksDone}/${progress.chunksTotal} ranges · ${progress.found} found`
                    : "Reading chain head…"}
                </span>
              )}
            </div>

            {/* Only while the scan is running: a bar left sitting at 100% reads as work still in
                progress, and the coverage box below already states the outcome. */}
            {scanning && progress && (
              <div
                className="h-1.5 w-full overflow-hidden rounded-full bg-surface-muted"
                role="progressbar"
                aria-valuemin={0}
                aria-valuemax={progress.chunksTotal}
                aria-valuenow={progress.chunksDone}
                aria-label="Chain scan progress"
              >
                <div
                  className="h-full bg-primary transition-all"
                  style={{
                    width: `${Math.round((progress.chunksDone / Math.max(1, progress.chunksTotal)) * 100)}%`,
                  }}
                />
              </div>
            )}

            {error && <p className="text-sm text-danger">{error}</p>}

            {result && (
              <DiscoveryResult
                result={result}
                ownSigners={ownSigners}
                foreignCount={foreignVerifications.length}
                onImport={() =>
                  navigate(
                    `/import?petId=${encodeURIComponent(petId)}&dogTagId=${encodeURIComponent(dogTagId)}`,
                  )
                }
                petName={petName}
              />
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}

/**
 * How a tag status is drawn. `revoked` is the only one that is a permanent, deliberate invalidation,
 * so it must not look identical to `lost` or `transfer pending`, which are lifecycle states a tag
 * recovers from. `unknown` is a byte the contract's enum does not define, and gets no colour that
 * would imply a meaning for it.
 */
const STATUS_VARIANT: Record<TagStatusLabel, "neutral" | "warning" | "danger"> = {
  active: "neutral",
  lost: "warning",
  "transfer pending": "warning",
  deceased: "warning",
  revoked: "danger",
  unknown: "neutral",
};

function DiscoveryResult({
  result,
  ownSigners,
  foreignCount,
  onImport,
  petName,
}: {
  result: TagDiscoveryResult;
  ownSigners: string[] | null;
  foreignCount: number;
  onImport: () => void;
  petName: string;
}) {
  const { coverage, profile } = result;
  const signersKnown = ownSigners !== null;
  return (
    <div className="space-y-4">
      {/* COVERAGE FIRST. Everything below is only as good as the window it was read from, so an
          incomplete scan says so before it shows a single row. */}
      <div
        className={`space-y-1 rounded-md border p-3 text-xs ${
          coverage.complete ? "border-border text-muted" : "border-warning/60 bg-warning/5 text-onSurface"
        }`}
        data-testid="discovery-coverage"
      >
        <p className="font-semibold uppercase tracking-wide">
          {coverage.complete
            ? "Scan complete"
            : coverage.cancelled
              ? "Scan cancelled — incomplete"
              : "Scan incomplete"}
        </p>
        {/*
          The REACHED extent, never the requested one. A cancelled scan that completed no chunk has
          read nothing, and printing the requested window as though it had been read is precisely the
          overstatement this whole panel exists to avoid.
        */}
        {coverage.reachedBlock === null ? (
          <p>
            No blocks were read (0 of {coverage.chunksTotal} ranges). The window asked for was{" "}
            {coverage.fromBlock.toString()}–{coverage.toBlock.toString()}.
          </p>
        ) : coverage.complete ? (
          <p>
            Read blocks {coverage.fromBlock.toString()}–{coverage.toBlock.toString()} of{" "}
            {coverage.latestBlock.toString()} ({coverage.chunksDone}/{coverage.chunksTotal} ranges).
          </p>
        ) : (
          <p>
            Reached back to block {coverage.reachedBlock.toString()} from{" "}
            {coverage.toBlock.toString()} ({coverage.chunksDone}/{coverage.chunksTotal} ranges). The
            window asked for was {coverage.fromBlock.toString()}–{coverage.toBlock.toString()}.
          </p>
        )}
        {!coverage.complete && (
          <p>
            {coverage.cancelled
              ? "You stopped the scan, so anything older was not read."
              : "Some ranges could not be read."}{" "}
            Anything in an unread range is UNKNOWN — not absent.
          </p>
        )}
        {coverage.failures.map((f) => (
          <p key={`${f.fromBlock}`} className="text-danger">
            blocks {f.fromBlock.toString()}–{f.toBlock.toString()} failed: {f.message}
          </p>
        ))}
      </div>

      {/* The tag itself — point-in-time reads at the chain head, not log-derived. */}
      <div className="space-y-2">
        <p className="text-xs font-semibold uppercase tracking-wide text-muted">
          The tag, read at the chain head
        </p>
        {profile ? (
          <div className="space-y-2 rounded-md border border-border p-3">
            <div className="flex flex-wrap items-center gap-2">
              <Badge variant={profile.valid ? "success" : profile.revoked ? "danger" : "warning"}>
                {profile.valid
                  ? "profile credential valid on chain"
                  : profile.revoked
                    ? "profile credential revoked on chain"
                    : "profile credential never anchored"}
              </Badge>
              <Badge variant={STATUS_VARIANT[result.statusLabel]}>
                tag status: {result.statusLabel}
              </Badge>
              {/* Only ever claimed against a KNOWN signer set. Unknown gets no badge here - the
                  notice below explains why no row on this page can be attributed. */}
              {attributeSigner(result.mintedBy, ownSigners) === "other" && (
                <Badge
                  variant="neutral"
                  title="The issuer that minted this tag is not one of this shop's signers, so this credential was created elsewhere."
                >
                  issued by another party
                </Badge>
              )}
            </div>
            <div className="grid gap-2 sm:grid-cols-2">
              <ChainValue label="credential root" value={profile.root} stacked />
              <ChainValue
                label="issuing contract"
                value={profile.issuerClone ?? undefined}
                href={addressExplorerHref(profile.issuerClone)}
                stacked
              />
              <ChainValue
                label="minted by (issuer)"
                value={result.mintedBy}
                href={addressExplorerHref(result.mintedBy)}
                stacked
              />
              <div className="flex flex-col gap-1">
                <span className="text-[11px] text-muted">anchored at</span>
                <ChainTime seconds={profile.issuedAt ? Number(profile.issuedAt) : null} />
              </div>
            </div>
            {!profile.issuerClone && (
              <p className="text-xs text-warning">
                The tag names this credential root, but the issuer factory does not know which contract
                anchored it — so its issuance state could not be checked here.
              </p>
            )}
          </div>
        ) : (
          <p className="text-sm text-muted">
            The chain holds no DogTag under this id. Either the tag was never minted, or the id
            recorded here is not this pet's.
          </p>
        )}
      </div>

      {/* Why a scan cannot list every credential — stated where the operator would otherwise expect
          to see them, not buried elsewhere. */}
      <p className="rounded-md border border-dashed border-border bg-surface-muted p-3 text-xs text-muted">
        A DogTag's other credentials — vaccinations, health attestations — are deliberately NOT linked
        to the tag id on chain, so no scan can list them: that link is what would turn a tag into a
        public lookup key for an animal's whole history. What the chain does show is the tag's own
        credential above and every verification below. A verification by someone other than this shop
        is evidence that such credentials exist — ask the owner to share them.
      </p>

      {/* Activity — every row a real log. */}
      <div className="space-y-2">
        <div className="flex flex-wrap items-center justify-between gap-2">
          <p className="text-xs font-semibold uppercase tracking-wide text-muted">
            On-chain activity for this tag
          </p>
          {/* The CTA follows from the finding "someone else verified this pet", so it cannot be
              offered when nothing was attributed. */}
          {signersKnown && foreignCount > 0 && (
            <Button size="sm" onClick={onImport}>
              Ask owner to share {foreignCount === 1 ? "the record" : "records"}
            </Button>
          )}
        </div>
        {/*
          Not knowing our own signers is reported as not knowing - never folded into "a stranger did
          this". The rows below are real logs either way; what is missing is only the attribution.
        */}
        {!signersKnown && (
          <p
            className="rounded-md border border-warning/60 bg-warning/5 p-3 text-xs text-onSurface"
            data-testid="discovery-signers-unknown"
          >
            This portal could not read its own signer list, so it cannot tell this shop's own
            verifications from anyone else's - every row below is left unattributed. The usual cause
            is custody being locked; unlock it and scan again to see which records are this shop's.
          </p>
        )}
        {result.events.length === 0 ? (
          <p className="text-sm text-muted">
            {coverage.complete
              ? "No activity for this tag in the blocks scanned."
              : "No activity found in the ranges that could be read — the rest of the window is unknown."}
          </p>
        ) : (
          <div className="space-y-2">
            {result.events.map((e) => (
              <EventRow
                key={`${e.txHash}-${e.kind}-${e.blockNumber}`}
                event={e}
                ownSigners={ownSigners}
              />
            ))}
          </div>
        )}
        {signersKnown && foreignCount > 0 && (
          <p className="text-xs text-muted">
            {foreignCount === 1
              ? "1 verification was recorded by someone other than this shop"
              : `${foreignCount} verifications were recorded by someone other than this shop`}
            , so {petName} holds at least one credential this shop has never seen. Importing is how the
            owner shares it.
          </p>
        )}
      </div>
    </div>
  );
}

const KIND_LABEL: Record<DiscoveredEvent["kind"], string> = {
  mint: "DogTag minted",
  statusChange: "Tag status changed",
  burn: "Tag erased",
  verification: "Verification recorded",
};

function EventRow({
  event,
  ownSigners,
}: {
  event: DiscoveredEvent;
  ownSigners: string[] | null;
}) {
  // Every row came out of a log the RPC returned, so the hash is real and the link always resolves.
  const href = txExplorerHref({ txHash: event.txHash });
  const actor =
    event.kind === "mint"
      ? event.issuer
      : event.kind === "statusChange"
        ? event.by
        : event.kind === "verification"
          ? event.relayer
          : null;
  // Tri-state: "another party" is a claim about a stranger and is only ever made against a signer
  // set this portal actually read. Not knowing renders as not knowing.
  const attribution = attributeSigner(actor, ownSigners);

  return (
    <div className="space-y-2 rounded-md border border-border p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm font-medium text-onSurface">{KIND_LABEL[event.kind]}</span>
        {actor && (
          <Badge
            variant={attribution === "own" ? "success" : "neutral"}
            title={
              attribution === "own"
                ? "Recorded by one of this shop's own signers."
                : attribution === "other"
                  ? "Recorded by an address that is not one of this shop's signers - this shop was not involved."
                  : "This portal could not read its own signer list, so it cannot say whether this was recorded by this shop or by someone else."
            }
          >
            {attribution === "own"
              ? "this shop"
              : attribution === "other"
                ? "another party"
                : "unknown"}
          </Badge>
        )}
        <span className="text-xs text-muted">block {event.blockNumber.toString()}</span>
      </div>
      <div className="grid gap-2 sm:grid-cols-2">
        <ChainValue
          label="transaction"
          value={event.txHash}
          href={href}
          stacked
          testId="discovery-tx"
        />
        <ChainValue
          label="contract"
          value={event.contract}
          href={addressExplorerHref(event.contract)}
          stacked
        />
        {event.kind === "mint" && (
          <ChainValue
            label="issuer"
            value={event.issuer}
            href={addressExplorerHref(event.issuer)}
            stacked
          />
        )}
        {event.kind === "statusChange" && (
          <>
            <ChainValue label="status" value={`${event.from} → ${event.to}`} stacked />
            <ChainValue
              label="changed by"
              value={event.by}
              href={addressExplorerHref(event.by)}
              stacked
            />
            {event.reason && <ChainValue label="reason" value={event.reason} stacked />}
          </>
        )}
        {event.kind === "verification" && (
          <>
            <ChainValue
              label="relayer (who checked)"
              value={event.relayer}
              href={addressExplorerHref(event.relayer)}
              stacked
            />
            {/* A HASHED key, and labelled as one: the chain carries no purpose word to print. */}
            <ChainValue label="purpose key" value={event.purposeKey} stacked />
            <ChainValue label="nullifier" value={event.nullifier} stacked />
            <div className="flex flex-col gap-1">
              <span className="text-[11px] text-muted">recorded at</span>
              <ChainTime seconds={Number(event.ts)} />
            </div>
          </>
        )}
      </div>
    </div>
  );
}

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
  discoverTag,
  isOwnSigner,
  resolveDogTagId,
  txExplorerHref,
  useToast,
  type DiscoveredEvent,
  type DiscoveryProgress,
  type TagDiscoveryResult,
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
  /** The shop's own signer addresses, so "was this us?" is a fact rather than a guess. */
  const [ownSigners, setOwnSigners] = useState<string[]>([]);

  const resolved = resolveDogTagId(dogTagId);

  // The shop's own signers come from the backend. If the read fails the list stays empty, which makes
  // every relayer render as "not this shop" — the safe direction, since it overstates what is
  // unfamiliar rather than claiming a stranger's verification as this shop's own work.
  useEffect(() => {
    let cancelled = false;
    void api
      .issuerSigners()
      .then((r) => {
        if (cancelled) return;
        // The whitelist matrix carries one row per (recordType, signer) pair, so the same address
        // appears repeatedly; only the distinct addresses matter here.
        const addrs = [r.activeSigner, ...(r.matrix ?? []).map((m) => m.address)].filter(Boolean);
        setOwnSigners([...new Set(signerAddress ? [...addrs, signerAddress] : addrs)]);
      })
      .catch(() => {
        if (!cancelled && signerAddress) setOwnSigners([signerAddress]);
      });
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
  const foreignVerifications = verifications.filter((v) => !isOwnSigner(v.relayer, ownSigners));

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
                    ? `Scanning blocks ${progress.scannedTo.toString()}–${progress.toBlock.toString()} · ${progress.chunksDone}/${progress.chunksTotal} ranges · ${progress.found} found`
                    : "Reading chain head…"}
                </span>
              )}
            </div>

            {progress && (
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

function DiscoveryResult({
  result,
  ownSigners,
  foreignCount,
  onImport,
  petName,
}: {
  result: TagDiscoveryResult;
  ownSigners: string[];
  foreignCount: number;
  onImport: () => void;
  petName: string;
}) {
  const { coverage, profile } = result;
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
        <p>
          Read blocks {coverage.fromBlock.toString()}–{coverage.toBlock.toString()} of{" "}
          {coverage.latestBlock.toString()} ({coverage.chunksDone}/{coverage.chunksTotal} ranges).
        </p>
        {!coverage.complete && (
          <p>
            {coverage.cancelled
              ? "You stopped the scan, so blocks below the range above were not read."
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
              <Badge variant={result.statusLabel === "active" ? "neutral" : "warning"}>
                tag status: {result.statusLabel}
              </Badge>
              {!isOwnSigner(result.mintedBy, ownSigners) && (
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
          {foreignCount > 0 && (
            <Button size="sm" onClick={onImport}>
              Ask owner to share {foreignCount === 1 ? "the record" : "records"}
            </Button>
          )}
        </div>
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
        {foreignCount > 0 && (
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
  ownSigners: string[];
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
  const mine = isOwnSigner(actor, ownSigners);

  return (
    <div className="space-y-2 rounded-md border border-border p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-sm font-medium text-onSurface">{KIND_LABEL[event.kind]}</span>
        {actor && (
          <Badge
            variant={mine ? "success" : "neutral"}
            title={
              mine
                ? "Recorded by one of this shop's own signers."
                : "Recorded by an address that is not one of this shop's signers — this shop was not involved."
            }
          >
            {mine ? "this shop" : "another party"}
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

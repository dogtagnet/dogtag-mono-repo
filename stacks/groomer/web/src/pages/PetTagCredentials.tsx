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
  resolveDogTagId,
  verifyCredentialOnchain,
  type CrmPet,
  type VerifyCredentialResp,
} from "@dogtag/ui";
import { FileCheck2, RefreshCw } from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Link } from "react-router-dom";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";

/**
 * The credentials this shop HOLDS for a pet, each checked against the chain right now.
 *
 * Two rules the rest of the product follows, applied here:
 *
 *  1. **Verified state comes from a real chain read.** Each document is re-verified through
 *     `verifyCredentialOnchain` — the same permissionless, direct-to-RPC path the ad-hoc verify panel
 *     uses (offline Merkle recompute + `issuedAt`/`isValid`/`isRevoked` on the issuing clone). The
 *     verdict recorded at import time is deliberately not shown: a root can be revoked the day after
 *     it was imported, so a stored verdict is a claim about the past presented as the present.
 *  2. **"Could not check" is never rendered as valid.** A failed read is its own state, visually
 *     distinct from both valid and invalid, and it says which check could not be made.
 *  3. **An unmatchable lookup is never rendered as an absence.** The held-document cache is keyed by
 *     the tag HANDLE (`POST /import/pull` files each document under its own
 *     `credentialSubject.dogTagId` leaf), while the link input deliberately also accepts the full
 *     on-chain field element so a tag copied from an explorer still scans. Handle -> field element is
 *     a hash and cannot be inverted, so a field-element link simply has no handle to look under. That
 *     is a lookup this shop cannot perform, not evidence it holds nothing - a surface built on not
 *     overstating must not understate either.
 */
export function PetTagCredentials({ pet }: { pet: CrmPet }) {
  const { api } = useApp();
  const [docs, setDocs] = useState<Record<string, unknown>[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // Reuses the discovery resolver rather than adding a second detector for the same two forms.
  const tagForm = resolveDogTagId(pet.dogTagId)?.form ?? null;

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const r = await api.petCredentials(pet.petId);
      setDocs(r.credentials);
      setError(null);
    } catch (e) {
      setError((e as Error).message);
    } finally {
      setLoading(false);
    }
  }, [api, pet.petId]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <Card>
      <CardHeader className="flex flex-row flex-wrap items-start justify-between gap-3">
        <div>
          <CardTitle className="flex items-center gap-2 text-base">
            <FileCheck2 className="h-4 w-4 text-primary" /> Credentials
          </CardTitle>
          <CardDescription>
            The credential documents this shop holds for {pet.name}, each re-checked against the chain
            now rather than trusting the verdict from when it was imported.
          </CardDescription>
        </div>
        <Button variant="outline" size="sm" onClick={() => void load()} disabled={loading}>
          <RefreshCw className="h-4 w-4" /> Refresh
        </Button>
      </CardHeader>
      <CardContent className="space-y-3">
        {loading && !docs ? (
          <div className="flex items-center gap-2 py-4 text-sm text-muted">
            <Spinner className="h-4 w-4" /> Loading credentials…
          </div>
        ) : error ? (
          <p className="text-sm text-danger">{error}</p>
        ) : !pet.dogTagId ? (
          <p className="text-sm text-muted">
            No DogTag is linked to {pet.name}, so there is no tag to look credentials up under. Link one
            above first.
          </p>
        ) : (docs?.length ?? 0) === 0 && tagForm === "field" ? (
          // NOT an absence. The lookup could not be performed at all, and saying "no credential" here
          // would be a false negative about this shop's own records.
          <p className="text-sm text-muted" data-testid="credentials-tag-form-mismatch">
            {pet.name} is linked by the full on-chain DogTag id{" "}
            <span className="font-mono">{pet.dogTagId}</span>, but credentials this shop imports are
            filed under the SHORT tag handle carried inside the credential itself. That handle is
            hashed to produce the on-chain id and the hash cannot be run backwards, so this shop's
            held records cannot be matched to this pet by the id above - which is not the same as
            holding none. Record the short number the owner's app shows to match them up, or{" "}
            <Link
              to={`/import?petId=${encodeURIComponent(pet.petId)}&dogTagId=${encodeURIComponent(pet.dogTagId)}`}
              className="text-primary hover:underline"
            >
              ask the owner to share a record
            </Link>
            . Chain discovery below is unaffected - it reads the on-chain id directly.
          </p>
        ) : (docs?.length ?? 0) === 0 ? (
          <p className="text-sm text-muted">
            This shop holds no credential for DogTag{" "}
            <span className="font-mono">{pet.dogTagId}</span>. That is a statement about THIS shop's
            records only — the owner may well hold credentials it has never shared.{" "}
            <Link
              to={`/import?petId=${encodeURIComponent(pet.petId)}&dogTagId=${encodeURIComponent(pet.dogTagId)}`}
              className="text-primary hover:underline"
            >
              Ask them to share one
            </Link>
            , or scan the chain below to see what exists.
          </p>
        ) : (
          docs?.map((doc, i) => (
            <CredentialRow key={credentialKey(doc, i)} doc={doc} />
          ))
        )}
      </CardContent>
    </Card>
  );
}

/** A stable React key: the doc's own Merkle root when it has one, else its position. */
function credentialKey(doc: Record<string, unknown>, index: number): string {
  const sig = doc.signature as { merkleRoot?: string } | undefined;
  return sig?.merkleRoot ?? `credential-${index}`;
}

/** What a single credential's chain check produced — or why it could not be made. */
type CheckState =
  | { phase: "checking" }
  | { phase: "checked"; result: VerifyCredentialResp }
  | { phase: "unavailable"; message: string };

function CredentialRow({ doc }: { doc: Record<string, unknown> }) {
  const [state, setState] = useState<CheckState>({ phase: "checking" });

  const issuer = (doc.issuer ?? {}) as { name?: string; recordType?: string; documentStore?: string };
  const signature = (doc.signature ?? {}) as { merkleRoot?: string };

  const check = useCallback(async () => {
    setState({ phase: "checking" });
    try {
      const result = await verifyCredentialOnchain({
        wrappedDoc: doc,
        rpcUrl: env.roaxRpc,
        registryAddr: env.issuerRegistryAddr || undefined,
      });
      setState({ phase: "checked", result });
    } catch (e) {
      // The chain read failed, so this credential's state is UNKNOWN. Rendering it as anything else —
      // valid, or invalid — would assert a fact that was never established.
      setState({ phase: "unavailable", message: (e as Error).message });
    }
  }, [doc]);

  useEffect(() => {
    void check();
  }, [check]);

  return (
    <div className="space-y-3 rounded-md border border-border p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-medium text-onSurface">
            {issuer.recordType || "credential"}
          </span>
          {issuer.name && <span className="text-xs text-muted">from {issuer.name}</span>}
        </div>
        <StatusBadge state={state} />
      </div>

      <div className="grid gap-2 sm:grid-cols-2">
        <ChainValue label="credential root" value={signature.merkleRoot} stacked />
        <ChainValue
          label="issuing contract"
          value={issuer.documentStore}
          href={addressExplorerHref(issuer.documentStore)}
          stacked
        />
        {state.phase === "checked" && (
          <>
            <div className="flex flex-col gap-1">
              <span className="text-[11px] text-muted">anchored on chain at</span>
              <ChainTime
                seconds={
                  state.result.issuedAt && state.result.issuedAt !== "0"
                    ? Number(state.result.issuedAt)
                    : null
                }
              />
            </div>
            <div className="flex flex-col gap-1">
              <span className="text-[11px] text-muted">chain checked at</span>
              <ChainTime seconds={state.result.checkedAt} />
            </div>
          </>
        )}
      </div>

      {state.phase === "checked" && <Fragments result={state.result} />}

      {state.phase === "unavailable" && (
        <div className="space-y-2 rounded-md border border-warning/60 bg-warning/5 p-3">
          <p className="text-xs text-onSurface">
            The chain could not be read, so this credential's issuance and revocation state are
            UNKNOWN — not valid, and not invalid. {state.message}
          </p>
          <Button variant="outline" size="sm" onClick={() => void check()}>
            <RefreshCw className="h-4 w-4" /> Check again
          </Button>
        </div>
      )}
    </div>
  );
}

function StatusBadge({ state }: { state: CheckState }) {
  if (state.phase === "checking") {
    return (
      <span className="inline-flex items-center gap-1.5 text-xs text-muted">
        <Spinner className="h-3 w-3" /> checking the chain…
      </span>
    );
  }
  if (state.phase === "unavailable") {
    // Never a valid-looking badge: an unread chain is an unknown, and it says so in those words.
    return (
      <Badge
        variant="warning"
        title="A chain read failed, so this credential's state could not be established. It is neither confirmed valid nor shown invalid."
      >
        could not check
      </Badge>
    );
  }
  const { verdict, status } = state.result;
  return (
    <Badge variant={verdict ? "success" : "danger"} title={`on-chain status: ${status}`}>
      {verdict ? "valid on chain" : status.replace(/_/g, " ")}
    </Badge>
  );
}

/**
 * The pillars behind the verdict, as the shared verify path reports them. Read straight from
 * `fragments` — the mapping is consumed, never re-derived here.
 */
function Fragments({ result }: { result: VerifyCredentialResp }) {
  const f = result.fragments;
  return (
    <div className="flex flex-wrap gap-2">
      <Pillar label="integrity" ok={f.integrity} />
      <Pillar label="anchored" ok={f.issued} />
      <Pillar label="not revoked" ok={!f.revoked} />
      {/* Absent or `null` means the whitelist pillar was not checked (there is no issuer signer
          address to check it for), which is distinct from checked-and-failed and must be drawn as
          neither a pass nor a fail. */}
      <Pillar label="issuer whitelisted" ok={f.issuerWhitelisted ?? null} />
    </div>
  );
}

function Pillar({ label, ok }: { label: string; ok: boolean | null }) {
  if (ok === null) {
    return (
      <Badge variant="neutral" title="Not checked on this path — no issuer signer address to check.">
        {label}: not checked
      </Badge>
    );
  }
  return (
    <Badge variant={ok ? "success" : "danger"}>
      {label}: {ok ? "pass" : "fail"}
    </Badge>
  );
}

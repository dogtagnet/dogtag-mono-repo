import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Input,
  Label,
  useToast,
  type FragmentState,
  type ImportVerdict,
  type IssuerResolution,
  type IssuerStoreAgreement,
  type IssuerWhitelistState,
} from "@dogtag/ui";
import type { CrmPet } from "@dogtag/ui";
import { CheckCircle2, HelpCircle, PawPrint, ScanLine, XCircle } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { useApp } from "../app/AppContext";

/**
 * Import a customer's pet PROFILE / VACCINATION credential (impl §5.2 / §3.5) — off-chain,
 * DECOUPLED from Verify. The customer app shows a QR carrying { userApiBase, userJwt, recordRef };
 * the groomer scans it, pulls the doc, and the backend third-party-verifies it on chain + DNS
 * (the three authenticity pillars). The verdict is shown before the record is accepted.
 */
export function ImportFromUser() {
  const { api } = useApp();
  const { toast } = useToast();
  const [params] = useSearchParams();
  const [kind, setKind] = useState<"profile" | "vaccination">("vaccination");
  const [userApiBase, setUserApiBase] = useState("");
  const [userJwt, setUserJwt] = useState("");
  const [recordRef, setRecordRef] = useState("");
  const [busy, setBusy] = useState(false);
  const [verdict, setVerdict] = useState<ImportVerdict | null>(null);
  const [accepted, setAccepted] = useState<boolean | null>(null);

  // Arriving from a pet's page (or its on-chain discovery panel) carries the pet across, so the
  // operator lands here already knowing WHOSE credential they are asking for instead of holding the
  // context in their head. Discovery names what exists; this is where the owner shares it.
  const petId = params.get("petId") ?? "";
  const contextTagId = params.get("dogTagId") ?? "";
  const [contextPet, setContextPet] = useState<CrmPet | null>(null);

  useEffect(() => {
    if (!petId) {
      setContextPet(null);
      return;
    }
    let cancelled = false;
    // A pet that cannot be read simply leaves the banner off; the import itself does not depend on it.
    void api
      .getPet(petId)
      .then((p) => {
        if (!cancelled) setContextPet(p);
      })
      .catch(() => {
        if (!cancelled) setContextPet(null);
      });
    return () => {
      cancelled = true;
    };
  }, [api, petId]);

  function tryParseScanned(text: string) {
    // a scanned QR may encode the whole payload as JSON; accept that too.
    try {
      const obj = JSON.parse(text) as Partial<{
        userApiBase: string;
        userJwt: string;
        recordRef: string;
      }>;
      if (obj.userApiBase) setUserApiBase(obj.userApiBase);
      if (obj.userJwt) setUserJwt(obj.userJwt);
      if (obj.recordRef) setRecordRef(obj.recordRef);
    } catch {
      /* not JSON — treat as a recordRef */
      setRecordRef(text);
    }
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    setVerdict(null);
    setAccepted(null);
    try {
      const r = await api.importPull({ userApiBase, userJwt, recordRef });
      setVerdict((r.verdict as ImportVerdict) ?? null);
      setAccepted(r.imported);
      toast({
        title: r.imported ? `${kind} accepted` : "Not accepted",
        description: r.imported
          ? "On-chain + DNS verification passed."
          : "Verification failed — the record was not accepted.",
        variant: r.imported ? "success" : "danger",
      });
    } catch (err) {
      // backend returns 422 with a verdict when verification fails; surface it.
      const body = (err as { body?: unknown }).body;
      if (body && typeof body === "object" && "verdict" in body) {
        setVerdict((body as { verdict: ImportVerdict }).verdict);
        setAccepted(false);
      }
      toast({ title: "Import failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ScanLine className="h-5 w-5 text-primary" /> Import from customer
        </CardTitle>
        <CardDescription>
          Pull a {kind} credential from the customer's app and verify it on chain + DNS BEFORE
          accepting (off-chain; decoupled from Verify).
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {(contextPet || contextTagId) && (
          <div className="flex flex-wrap items-center justify-between gap-2 rounded-md border border-primary/40 bg-primary/5 p-3 text-sm">
            <span className="flex flex-wrap items-center gap-2 text-onSurface">
              <PawPrint className="h-4 w-4 text-primary" />
              Importing for{" "}
              {contextPet ? (
                <Link to={`/pets/${contextPet.petId}`} className="font-medium text-primary hover:underline">
                  {contextPet.name}
                </Link>
              ) : (
                "this pet"
              )}
              {contextPet?.clientName && (
                <>
                  {" · owner "}
                  <Link
                    to={`/clients/${contextPet.clientId}`}
                    className="text-primary hover:underline"
                  >
                    {contextPet.clientName}
                  </Link>
                </>
              )}
              {(contextPet?.dogTagId || contextTagId) && (
                <Badge variant="outline" className="font-mono">
                  DogTag {contextPet?.dogTagId || contextTagId}
                </Badge>
              )}
            </span>
            {/*
              The tag is shown, not sent: `POST /import/pull` files an accepted document under the
              dogTagId carried INSIDE the credential itself. Passing this one as a hint would let a
              mis-scanned QR be filed against the wrong pet, so the credential stays the authority on
              whose it is.
            */}
            <span className="text-xs text-muted">
              The record is filed under the tag inside the credential, not under this one.
            </span>
          </div>
        )}

        <div className="flex gap-2">
          <Button
            variant={kind === "profile" ? "primary" : "outline"}
            size="sm"
            onClick={() => setKind("profile")}
          >
            Import Profile
          </Button>
          <Button
            variant={kind === "vaccination" ? "primary" : "outline"}
            size="sm"
            onClick={() => setKind("vaccination")}
          >
            Import Vaccination
          </Button>
        </div>

        <div className="rounded-lg border border-dashed border-border bg-surface-muted p-4 text-sm text-muted">
          Ask the customer to open their DogTag app and present the share QR. Scan it, or paste the
          scanned payload / record reference below.
        </div>

        <div className="space-y-1.5">
          <Label>Scanned payload</Label>
          <Input
            placeholder="Paste scanned QR contents…"
            onChange={(e) => tryParseScanned(e.target.value)}
          />
        </div>

        <form onSubmit={submit} className="grid gap-4 sm:grid-cols-2">
          <div className="space-y-1.5">
            <Label required>Customer API base</Label>
            <Input
              value={userApiBase}
              onChange={(e) => setUserApiBase(e.target.value)}
              required
              placeholder="https://api.dogtag.io"
            />
          </div>
          <div className="space-y-1.5">
            <Label required>Record reference</Label>
            <Input value={recordRef} onChange={(e) => setRecordRef(e.target.value)} required />
          </div>
          <div className="space-y-1.5 sm:col-span-2">
            <Label required>Customer JWT</Label>
            <Input value={userJwt} onChange={(e) => setUserJwt(e.target.value)} required />
          </div>
          <div className="sm:col-span-2">
            <Button type="submit" loading={busy}>
              Pull &amp; verify
            </Button>
          </div>
        </form>

        {verdict && <VerdictPanel verdict={verdict} accepted={accepted} />}
      </CardContent>
    </Card>
  );
}

function VerdictPanel({
  verdict,
  accepted,
}: {
  verdict: ImportVerdict;
  accepted: boolean | null;
}) {
  return (
    <div className="space-y-3 rounded-lg border border-border p-4">
      <div className="flex items-center justify-between">
        <span className="text-sm font-semibold text-onSurface">Verification verdict</span>
        <Badge variant={verdict.valid ? "success" : "danger"}>
          {verdict.valid ? "VALID" : "INVALID"}
        </Badge>
      </div>
      <p className="text-xs text-muted">
        The authenticity pillars define validity for everyone. <em>Ownership</em> is a contextual
        fragment — <code>NOT_APPLICABLE</code> for a third-party groomer importing a customer's
        record.
      </p>
      <div className="grid gap-2 sm:grid-cols-2">
        <Pillar label="Integrity" state={verdict.integrity} />
        <Pillar label="Issuance" state={verdict.issuance} />
        <Pillar label="Identity (DNS)" state={verdict.identity} />
        <Pillar label="Ownership" state={verdict.ownership} />
        {/*
          Rendered even though it is not a FragmentState, because without it a credential refused
          SOLELY by this pillar would show four green tiles beside an INVALID badge with nothing
          explaining why. Its four states are kept distinct for the same reason the wire keeps them
          distinct: "not evaluated" must never look like "passed".
        */}
        <IssuerWhitelistPillar
          state={verdict.issuerWhitelistState}
          resolution={verdict.issuerResolution}
          store={verdict.issuerStoreAgreement}
        />
      </div>
      {accepted !== null && (
        <p className="text-sm">
          {accepted ? (
            <span className="text-success">Record accepted and imported.</span>
          ) : (
            <span className="text-danger">Record rejected — not imported.</span>
          )}
        </p>
      )}
    </div>
  );
}

/**
 * The factory-anchored issuer-whitelist pillar, plus the separate documentStore-agreement term.
 *
 * `Unresolved` reads as a WARNING rather than a neutral tile on purpose: an indeterminate pillar is
 * a failure to establish a claim, not a step that was harmlessly skipped, and it does fail the
 * import. `Not checked` is reserved for the one case that genuinely did not run and genuinely is not
 * evidence — this deployment having no `FACTORY_ADDR` — and it says so, rather than going quiet.
 *
 * A `differs` store is surfaced as its OWN refusal rather than recoloured into the pillar, because
 * "the document named a contract the chain did not" sends the operator somewhere different from "the
 * signer is not authorised". The backend keeps the two apart for the same reason.
 *
 * Every field is optional: the backend is deployed separately, so a portal newer than its API must
 * degrade to an honest unknown instead of throwing on a missing key and blanking the whole panel.
 */
function IssuerWhitelistPillar({
  state,
  resolution,
  store,
}: {
  state?: IssuerWhitelistState;
  resolution?: IssuerResolution;
  store?: IssuerStoreAgreement;
}) {
  type Tile = {
    variant: "success" | "danger" | "warning" | "neutral";
    icon: typeof CheckCircle2;
    text: string;
  };
  const map: Record<IssuerWhitelistState, Tile> = {
    passed: { variant: "success", icon: CheckCircle2, text: "AUTHORISED" },
    failed: { variant: "danger", icon: XCircle, text: "NOT AUTHORISED" },
    unresolved: { variant: "warning", icon: HelpCircle, text: "UNRESOLVED" },
    unavailableNoFactoryConfigured: {
      variant: "neutral",
      icon: HelpCircle,
      text: "NOT CHECKED",
    },
  };
  // Deliberately NOT the `NOT CHECKED` copy: that asserts the specific no-factory case, which a
  // backend that reported nothing at all has told us nothing about.
  const unreported: Tile = { variant: "warning", icon: HelpCircle, text: "UNKNOWN" };
  // A store mismatch outranks the pillar in the headline, because it is the stronger and more
  // specific accusation — the chain named a different contract than the document did.
  const storeMismatch = store === "differs";
  const mismatch: Tile = { variant: "danger", icon: XCircle, text: "ISSUER MISMATCH" };
  const tile: Tile = storeMismatch ? mismatch : state ? map[state] : unreported;
  const { variant, icon: Icon, text } = tile;
  const why = storeMismatch
    ? "the document names a contract the chain did not"
    : !state
      ? "this backend did not report the issuer pillar"
      : state === "unavailableNoFactoryConfigured"
        ? "this verifier has no factory configured"
        : resolution === "noRecord"
          ? "no factory clone claims this root"
          : resolution === "readFailed"
            ? "the anchor could not be read"
            : null;
  return (
    <div className="flex items-center justify-between gap-2 rounded-md border border-border px-3 py-2">
      <span className="text-sm text-onSurface">
        Issuer whitelist
        {why && <span className="block text-xs text-muted">{why}</span>}
      </span>
      <Badge variant={variant}>
        <Icon className="h-3 w-3" />
        {text}
      </Badge>
    </div>
  );
}

function Pillar({ label, state }: { label: string; state: FragmentState }) {
  const map: Record<
    FragmentState,
    { variant: "success" | "danger" | "warning" | "neutral"; icon: typeof CheckCircle2 }
  > = {
    VALID: { variant: "success", icon: CheckCircle2 },
    INVALID: { variant: "danger", icon: XCircle },
    ERROR: { variant: "warning", icon: HelpCircle },
    NOT_APPLICABLE: { variant: "neutral", icon: HelpCircle },
  };
  const { variant, icon: Icon } = map[state];
  return (
    <div className="flex items-center justify-between gap-2 rounded-md border border-border px-3 py-2">
      <span className="text-sm text-onSurface">{label}</span>
      <Badge variant={variant}>
        <Icon className="h-3 w-3" />
        {state}
      </Badge>
    </div>
  );
}

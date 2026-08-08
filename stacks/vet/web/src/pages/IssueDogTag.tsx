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
  QrCode,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  explorerTxUrl,
  useToast,
  type MicrochipStandard,
  type NeuterStatus,
  type PetSex,
  type ProfileIssueStartResp,
  type ProfileIssueStatusResp,
  type ProfileMicrochip,
  type ProfileOwnerIdentity,
  type ProfilePet,
} from "@dogtag/ui";
import { AlertTriangle, CheckCircle2, Sparkles } from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  AnchorBlockedCard,
  AnchorUnknownNotice,
  useDogTagAnchorReadiness,
} from "../app/AnchorReadiness";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";

const POLL_MS = 2_000;
/**
 * Consecutive polls that must report the token dead (server's clock, `tokenSecondsLeft === 0`,
 * session still pending) before the card declares it. The grace absorbs the race where a device's
 * bind consumed the token in its final seconds and the row is about to flip to "minting" — a
 * single observation would declare dead an issuance that is already anchoring.
 */
const DEAD_POLLS = 3;

interface OwnerForm {
  countryOfIdentification: string;
  identification: string;
  name: string;
}
interface PetForm {
  name: string;
  species: string;
  breedVbo: string;
  breedLabel: string;
  sex: string;
  neuterStatus: string;
  dateOfBirth: string;
  microchipCode: string;
  microchipStandard: string;
  microchipImplantDate: string;
}

const EMPTY_OWNER: OwnerForm = { countryOfIdentification: "", identification: "", name: "" };
const EMPTY_PET: PetForm = {
  name: "",
  species: "",
  breedVbo: "",
  breedLabel: "",
  sex: "",
  neuterStatus: "",
  dateOfBirth: "",
  microchipCode: "",
  microchipStandard: "",
  microchipImplantDate: "",
};

const DEMO_OWNER: OwnerForm = {
  countryOfIdentification: "GB",
  identification: "P1234567",
  name: "Alex Doe",
};
const DEMO_PET: PetForm = {
  name: "Rex",
  species: "dog",
  breedVbo: "",
  breedLabel: "Labrador Retriever",
  sex: "male",
  neuterStatus: "neutered",
  dateOfBirth: "2021-04-15",
  microchipCode: "900123456789012",
  microchipStandard: "ISO_11784_11785",
  microchipImplantDate: "2021-06-01",
};

const SEX_OPTIONS: { value: PetSex; label: string }[] = [
  { value: "male", label: "Male" },
  { value: "female", label: "Female" },
];
const NEUTER_OPTIONS: { value: NeuterStatus; label: string }[] = [
  { value: "intact", label: "Intact" },
  { value: "neutered", label: "Neutered" },
  { value: "spayed", label: "Spayed" },
];
const MICROCHIP_STANDARD_OPTIONS: { value: MicrochipStandard; label: string }[] = [
  { value: "ISO_11784_11785", label: "ISO 11784/11785" },
  { value: "OTHER", label: "Other" },
];

export function IssueDogTag() {
  const { api, signingMode } = useApp();
  const { toast } = useToast();
  // Whether this backend can ANCHOR a dog tag (config facts off /health). `blocked` replaces the
  // form: allocating a tag and having the owner scan a QR for an issuance the backend already knows
  // it cannot complete is the defect this screen used to ship. `unknown` only warns — could-not-check
  // is not a refusal, and the backend refuses an unanchorable start itself, allocating nothing.
  const anchor = useDogTagAnchorReadiness();

  // Testnet demo: prefill a valid owner + pet so you just click Start issuance (no typing).
  // Production (demo off): forms start empty.
  const [owner, setOwner] = useState<OwnerForm>(() => (env.demoMode ? { ...DEMO_OWNER } : { ...EMPTY_OWNER }));
  const [pet, setPet] = useState<PetForm>(() => (env.demoMode ? { ...DEMO_PET } : { ...EMPTY_PET }));
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);

  const [session, setSession] = useState<ProfileIssueStartResp | null>(null);
  const [status, setStatus] = useState<ProfileIssueStatusResp | null>(null);
  // A refused start, rendered IN PLACE under the form (a toast scrolls away; the operator is
  // standing here). The backend refuses BEFORE allocating a tag or drawing a QR when the clinic's
  // signing key definitively may not issue — the refusal names which half is missing and which
  // portal fixes it, so the message is the server's own words.
  const [startRefusal, setStartRefusal] = useState<string | null>(null);
  // Consecutive polls on which the SERVER reported the token dead while the session was still
  // pending. The portal holds no deadline clock of its own: it once declared "expired" from a
  // hardcoded 180s while the backend would still have accepted a bind — and showed the same
  // "expired" for a phone that never reached this machine at all.
  const [deadPolls, setDeadPolls] = useState(0);

  function fillDemo() {
    setOwner({ ...DEMO_OWNER });
    setPet({ ...DEMO_PET });
    setErrors({});
    toast({ title: "Demo data filled", description: "Valid owner + pet — review and Start issuance.", variant: "success" });
  }

  function setOwnerField(key: keyof OwnerForm, v: string) {
    setOwner((prev) => ({ ...prev, [key]: v }));
    setErrors((prev) => {
      const { [`owner.${key}`]: _drop, ...rest } = prev;
      return rest;
    });
  }
  function setPetField(key: keyof PetForm, v: string) {
    setPet((prev) => ({ ...prev, [key]: v }));
    setErrors((prev) => {
      const { [`pet.${key}`]: _drop, ...rest } = prev;
      return rest;
    });
  }

  function validate(): boolean {
    const next: Record<string, string> = {};
    if (!owner.countryOfIdentification.trim()) next["owner.countryOfIdentification"] = "Required";
    if (!owner.identification.trim()) next["owner.identification"] = "Required";
    if (!owner.name.trim()) next["owner.name"] = "Required";
    if (!pet.name.trim()) next["pet.name"] = "Pet name is required";
    if (pet.microchipCode.trim() && !pet.microchipStandard)
      next["pet.microchipStandard"] = "Standard required when a chip code is provided";
    setErrors(next);
    return Object.keys(next).length === 0;
  }

  function buildBody(): { ownerIdentity: ProfileOwnerIdentity; pet: ProfilePet } {
    const ownerIdentity: ProfileOwnerIdentity = {
      countryOfIdentification: owner.countryOfIdentification.trim(),
      identification: owner.identification.trim(),
      name: owner.name.trim(),
    };
    const petBody: ProfilePet = { name: pet.name.trim() };
    if (pet.species.trim()) petBody.species = pet.species.trim();
    if (pet.breedVbo.trim()) petBody.breedVbo = pet.breedVbo.trim();
    if (pet.breedLabel.trim()) petBody.breedLabel = pet.breedLabel.trim();
    if (pet.sex) petBody.sex = pet.sex as PetSex;
    if (pet.neuterStatus) petBody.neuterStatus = pet.neuterStatus as NeuterStatus;
    if (pet.dateOfBirth.trim()) petBody.dateOfBirth = pet.dateOfBirth.trim();
    if (pet.microchipCode.trim()) {
      const microchip: ProfileMicrochip = {
        code: pet.microchipCode.trim(),
        standard: (pet.microchipStandard || "ISO_11784_11785") as MicrochipStandard,
      };
      if (pet.microchipImplantDate.trim()) microchip.implantDate = pet.microchipImplantDate.trim();
      petBody.microchip = microchip;
    }
    return { ownerIdentity, pet: petBody };
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    if (!validate()) {
      toast({ title: "Please fix the highlighted fields", variant: "danger" });
      return;
    }
    setBusy(true);
    setStartRefusal(null);
    try {
      const resp = await api.startProfileIssue(buildBody());
      setSession(resp);
      setStatus(null);
      setDeadPolls(0);
      toast({ title: "Issuance started", description: `Dog tag ${resp.dogTagId} allocated.`, variant: "success" });
    } catch (err) {
      setStartRefusal((err as Error).message);
      toast({ title: "Start failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  function restart() {
    setSession(null);
    setStatus(null);
    setDeadPolls(0);
  }

  const bound = status?.status === "bound";
  const errored = status?.status === "error";

  // Poll the session status every ~2s until a terminal state. There is deliberately NO local
  // deadline: what the card says comes from the server's own facts (status, resolvedAt,
  // tokenSecondsLeft, qrAddress), so it can tell "nobody picked this QR up" from "a device
  // resolved it and went quiet" from "the deadline passed" — three different faults with three
  // different remedies. Polling continues even after the token dies, so a bind that landed in the
  // token's final seconds still flips the card to "anchoring" instead of being declared dead.
  // An errored session is a definite terminal answer — polling past it can only overwrite the
  // reason with nothing.
  const deadPollsRef = useRef(0);
  useEffect(() => {
    if (!session || bound || errored) return;
    deadPollsRef.current = 0;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;

    const tick = async () => {
      if (cancelled) return;
      try {
        const s = await api.profileIssueStatus(session.sessionId);
        if (cancelled) return;
        setStatus(s);
        if (s.status === "bound" || s.status === "error") return; // effect re-runs; terminal
        const dead = s.status === "pending" && s.tokenSecondsLeft !== undefined && s.tokenSecondsLeft <= 0;
        deadPollsRef.current = dead ? deadPollsRef.current + 1 : 0;
        setDeadPolls(deadPollsRef.current);
      } catch {
        /* transient — keep polling; a fetch failure is not evidence about the session */
      }
      if (!cancelled) timer = setTimeout(tick, POLL_MS);
    };
    timer = setTimeout(tick, POLL_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [session, bound, errored, api]);

  // Re-arm a FAILED issuance: same session, same dogTagId, same identity salts, fresh QR. This is
  // the ONLY path that can complete a stranded (anchored, unminted) root — a fresh session
  // allocates a new dogTagId and the device derives a DIFFERENT root from it.
  async function retry() {
    if (!session) return;
    setBusy(true);
    try {
      const resp = await api.retryProfileIssue(session.sessionId);
      setSession(resp);
      setStatus(null);
      setDeadPolls(0);
      toast({
        title: "Issuance re-armed",
        description: `Fresh QR for dog tag ${resp.dogTagId} — the owner scans again.`,
        variant: "success",
      });
    } catch (err) {
      toast({ title: "Retry refused", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  if (session && bound && status) {
    return (
      <div className="space-y-4">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <CheckCircle2 className="h-5 w-5 text-success" /> Dog tag issued
            </CardTitle>
            <CardDescription>The owner's device bound this dog tag on-chain.</CardDescription>
          </CardHeader>
          <CardContent className="space-y-4">
            <DetailRow label="Dog tag id" value={status.dogTagId} />
            {status.walletAddress && <DetailRow label="Owner wallet" value={status.walletAddress} mono />}
            {status.root && <DetailRow label="Merkle root" value={status.root} mono />}
            {status.txHash && (
              <div className="space-y-1.5">
                <Label>Transaction</Label>
                <a
                  href={explorerTxUrl(status.txHash)}
                  target="_blank"
                  rel="noreferrer"
                  className="block break-all font-mono text-sm text-primary underline"
                >
                  {status.txHash}
                </a>
              </div>
            )}
            <Button variant="ghost" onClick={restart}>
              Issue another
            </Button>
          </CardContent>
        </Card>
      </div>
    );
  }

  // A FAILED issuance is a definite answer and renders as one — never as "the QR token expired",
  // and never as a silent stall. The reason is the backend's own operator-vocabulary sentence
  // (`error`), and Retry re-arms THIS session because a stranded (anchored, unminted) root can
  // only ever be completed through it.
  if (session && errored && status) {
    return (
      <Card data-testid="issuance-error">
        <CardHeader>
          <CardTitle className="flex items-center gap-2 text-danger">
            <AlertTriangle className="h-5 w-5" /> Issuance failed
          </CardTitle>
          <CardDescription>
            Dog tag <Badge variant="neutral">{status.dogTagId || session.dogTagId}</Badge> — the
            owner's device bound, but the on-chain issuance did not complete
            {status.errorStage ? ` (failed at: ${status.errorStage})` : ""}.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-3">
          <p
            data-testid="issuance-error-reason"
            className="rounded-lg border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-onSurface"
          >
            {status.error ||
              "The backend did not record a reason. Check the vet backend's log for the failing stage."}
          </p>
          <div className="flex gap-2">
            <Button onClick={retry} loading={busy} data-testid="issuance-retry">
              Retry this issuance
            </Button>
            <Button variant="ghost" onClick={restart}>
              Start over
            </Button>
          </div>
          <p className="text-xs text-muted">
            Retry keeps the same dog tag and shows a fresh QR for the owner to scan — if the root
            was already anchored on-chain, the retry completes the remaining mint instead of
            re-anchoring. Start over abandons this session and allocates a new dog tag.
          </p>
        </CardContent>
      </Card>
    );
  }

  if (session) {
    // Everything this card claims comes from the SERVER's facts, never a local clock.
    const qrAddr = status?.qrAddress ?? session.qrAddress;
    const mismatch = qrAddr?.check === "notSelfAddressed" ? qrAddr : null;
    const resolved = Boolean(status?.resolvedAt);
    const anchoring = status?.status === "minting";
    const tokenDead = deadPolls >= DEAD_POLLS;
    const secondsLeft = status?.tokenSecondsLeft ?? session.ttlSecs ?? null;

    // The machine no longer answers at the address printed inside the QR: phones post into the
    // void and this backend's log records nothing — shown wherever the QR is shown, because it is
    // the one fault the operator can neither see nor debug from this screen.
    const mismatchNotice = mismatch && (
      <div
        data-testid="qr-address-mismatch"
        className="max-w-sm rounded-md border border-danger/40 bg-danger/10 p-3 text-sm text-danger"
      >
        Phones cannot reach this QR: it points at <span className="font-mono">{mismatch.host}</span>,
        but this machine now answers at <span className="font-mono">{mismatch.currentAddress}</span> —
        the network address changed after the vet stack started. Restart the vet stack so new QRs
        carry the current address, then draw a new QR.
      </div>
    );

    return (
      <Card>
        <CardHeader>
          <CardTitle>Owner scans to receive their dog tag</CardTitle>
          <CardDescription>
            Dog tag <Badge variant="neutral">{session.dogTagId}</Badge> allocated — pending device bind.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {anchoring ? (
            <div data-testid="qr-anchoring" className="flex flex-col items-center gap-3">
              <p className="max-w-sm text-center text-sm text-onSurface">
                The owner's device sent its profile — the dog tag is being anchored on-chain now.
                This can take a minute or two; leave this page open.
              </p>
              <Badge variant="neutral">Status: anchoring on-chain</Badge>
            </div>
          ) : tokenDead && !resolved ? (
            <div data-testid="qr-dead-unclaimed" className="space-y-3 text-center">
              <p className="mx-auto max-w-sm text-sm text-danger">
                No device ever picked this QR up before it expired. If the owner did scan it, their
                phone could not reach this machine
                {qrAddr?.host ? (
                  <>
                    {" "}
                    at <span className="font-mono">{qrAddr.host}</span>
                  </>
                ) : null}
                {" — check both are on the same network, then draw a new QR."}
              </p>
              <div className="flex flex-col items-center gap-3">{mismatchNotice}</div>
              <Button onClick={restart}>Start over</Button>
            </div>
          ) : tokenDead && resolved ? (
            <div data-testid="qr-dead-after-pickup" className="space-y-3 text-center">
              <p className="mx-auto max-w-sm text-sm text-danger">
                The owner's device picked this QR up but never sent its profile before the deadline —
                the phone may have lost its connection to this machine, or the owner stopped partway.
                Draw a new QR and have the owner scan again.
              </p>
              <div className="flex flex-col items-center gap-3">{mismatchNotice}</div>
              <Button onClick={restart}>Start over</Button>
            </div>
          ) : (
            <div className="flex flex-col items-center gap-3">
              {mismatchNotice}
              {session.signerIssuance?.state === "unknown" && (
                // Could-not-check WARNS and never refuses: the backend could not establish the
                // signing key's issue approval, so the QR is still offered and the later failure's
                // remedies are named up front.
                <div
                  data-testid="issue-gate-unknown"
                  className="max-w-sm rounded-md border border-warning/40 bg-warning/10 p-3 text-sm text-onSurface"
                >
                  Could not check whether this clinic's signing key may issue dog tags:{" "}
                  {session.signerIssuance.detail}
                </div>
              )}
              <QrCode value={session.qr} caption={session.qr} />
              {resolved ? (
                <p data-testid="qr-picked-up" className="max-w-sm text-center text-sm text-muted">
                  The owner's device has picked this up — it is building the private owner profile
                  and sending it to this clinic. Waiting for the profile…
                </p>
              ) : (
                <p data-testid="qr-waiting" className="max-w-sm text-center text-sm text-muted">
                  Owner scans this in the DogTag app to receive their dog tag. Waiting for a device
                  to scan…
                </p>
              )}
              {secondsLeft !== null && (
                // Counted from the server's ttl at response receipt (refreshed every poll) — never
                // from a browser-side deadline, whose clock the backend does not share.
                <Badge variant="neutral" data-testid="qr-seconds-left">
                  QR valid for about {Math.floor(secondsLeft / 60)}m {secondsLeft % 60}s
                </Badge>
              )}
              <Button variant="ghost" size="sm" onClick={restart}>
                Cancel / start over
              </Button>
            </div>
          )}
        </CardContent>
      </Card>
    );
  }

  // A DEFINITE "this backend cannot anchor" answer refuses IN PLACE: no form, no tag allocated,
  // no QR for a second human to scan. Only `blocked` may do this — see `useDogTagAnchorReadiness`.
  if (anchor?.state === "blocked") {
    return <AnchorBlockedCard readiness={anchor} />;
  }

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between gap-4">
        <div>
          <CardTitle>Register pet (issue dog tag)</CardTitle>
          <CardDescription>
            Do this <strong>once per pet</strong>, before issuing any records for it. Signing mode:{" "}
            <Badge variant="neutral">{signingMode}</Badge> — capture the owner identity + pet profile,
            then the owner's device binds the dog tag on-chain.
          </CardDescription>
        </div>
        {env.demoMode && (
          <Button type="button" variant="outline" size="sm" onClick={fillDemo}>
            <Sparkles className="h-4 w-4" /> Fill demo data
          </Button>
        )}
      </CardHeader>
      <CardContent className="space-y-4">
        {anchor && <AnchorUnknownNotice readiness={anchor} />}
        <form onSubmit={submit} className="space-y-6">
          <fieldset className="space-y-3">
            <legend className="text-sm font-semibold text-onSurface">Owner identity</legend>
            <div className="grid gap-4 sm:grid-cols-2">
              <TextField
                label="Country of identification"
                required
                value={owner.countryOfIdentification}
                error={errors["owner.countryOfIdentification"]}
                placeholder="GB"
                onChange={(v) => setOwnerField("countryOfIdentification", v)}
              />
              <TextField
                label="ID / passport number"
                required
                value={owner.identification}
                error={errors["owner.identification"]}
                placeholder="P1234567"
                onChange={(v) => setOwnerField("identification", v)}
              />
              <TextField
                label="Owner name (as on ID)"
                required
                value={owner.name}
                error={errors["owner.name"]}
                placeholder="Alex Doe"
                onChange={(v) => setOwnerField("name", v)}
              />
            </div>
          </fieldset>

          <fieldset className="space-y-3">
            <legend className="text-sm font-semibold text-onSurface">Pet profile</legend>
            <div className="grid gap-4 sm:grid-cols-2">
              <TextField
                label="Name"
                required
                value={pet.name}
                error={errors["pet.name"]}
                placeholder="Rex"
                onChange={(v) => setPetField("name", v)}
              />
              <TextField
                label="Species"
                value={pet.species}
                placeholder="dog"
                onChange={(v) => setPetField("species", v)}
              />
              <TextField
                label="Breed (VBO code)"
                value={pet.breedVbo}
                placeholder="optional"
                onChange={(v) => setPetField("breedVbo", v)}
              />
              <TextField
                label="Breed (label)"
                value={pet.breedLabel}
                placeholder="Labrador Retriever"
                onChange={(v) => setPetField("breedLabel", v)}
              />
              <SelectField
                label="Sex"
                value={pet.sex}
                options={SEX_OPTIONS}
                onChange={(v) => setPetField("sex", v)}
              />
              <SelectField
                label="Neuter status"
                value={pet.neuterStatus}
                options={NEUTER_OPTIONS}
                onChange={(v) => setPetField("neuterStatus", v)}
              />
              <div className="space-y-1.5">
                <Label>Date of birth</Label>
                <Input
                  type="date"
                  value={pet.dateOfBirth}
                  onChange={(e) => setPetField("dateOfBirth", e.target.value)}
                />
              </div>
            </div>
          </fieldset>

          <fieldset className="space-y-3">
            <legend className="text-sm font-semibold text-onSurface">Microchip (optional)</legend>
            <div className="grid gap-4 sm:grid-cols-2">
              <TextField
                label="Chip code"
                value={pet.microchipCode}
                placeholder="900123456789012"
                onChange={(v) => setPetField("microchipCode", v)}
              />
              <SelectField
                label="Standard"
                value={pet.microchipStandard}
                error={errors["pet.microchipStandard"]}
                options={MICROCHIP_STANDARD_OPTIONS}
                onChange={(v) => setPetField("microchipStandard", v)}
              />
              <div className="space-y-1.5">
                <Label>Implant date</Label>
                <Input
                  type="date"
                  value={pet.microchipImplantDate}
                  onChange={(e) => setPetField("microchipImplantDate", e.target.value)}
                />
              </div>
            </div>
          </fieldset>

          <Button type="submit" loading={busy} disabled={anchor === null}>
            Start issuance
          </Button>
          {anchor === null && (
            // A disabled control renders its reason: the readiness probe has not answered yet.
            <p className="text-xs text-muted">Checking whether this backend can anchor dog tags…</p>
          )}
          {startRefusal && (
            <div
              data-testid="issue-gate-refusal"
              className="rounded-md border border-danger/40 bg-danger/10 p-3 text-sm text-danger"
            >
              {startRefusal}
            </div>
          )}
        </form>
      </CardContent>
    </Card>
  );
}

function DetailRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="space-y-1.5">
      <Label>{label}</Label>
      <p className={mono ? "break-all font-mono text-sm text-onSurface" : "text-sm text-onSurface"}>{value}</p>
    </div>
  );
}

function TextField({
  label,
  value,
  onChange,
  required,
  error,
  placeholder,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  required?: boolean;
  error?: string;
  placeholder?: string;
}) {
  return (
    <div className="space-y-1.5">
      <Label required={required}>{label}</Label>
      <Input
        value={value}
        placeholder={placeholder}
        invalid={Boolean(error)}
        onChange={(e) => onChange(e.target.value)}
      />
      {error && <p className="text-xs text-danger">{error}</p>}
    </div>
  );
}

function SelectField({
  label,
  value,
  onChange,
  options,
  error,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
  error?: string;
}) {
  return (
    <div className="space-y-1.5">
      <Label>{label}</Label>
      <Select value={value} onValueChange={onChange}>
        <SelectTrigger>
          <SelectValue placeholder="Choose…" />
        </SelectTrigger>
        <SelectContent>
          {options.map((o) => (
            <SelectItem key={o.value} value={o.value}>
              {o.label}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {error && <p className="text-xs text-danger">{error}</p>}
    </div>
  );
}

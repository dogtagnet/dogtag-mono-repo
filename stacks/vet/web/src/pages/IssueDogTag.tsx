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
import { CheckCircle2, Sparkles } from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";

/** Token expiry the backend enforces (180s). After this the device can no longer bind. */
const TOKEN_TTL_MS = 180_000;
const POLL_MS = 2_000;

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

  // Testnet demo: prefill a valid owner + pet so you just click Start issuance (no typing).
  // Production (demo off): forms start empty.
  const [owner, setOwner] = useState<OwnerForm>(() => (env.demoMode ? { ...DEMO_OWNER } : { ...EMPTY_OWNER }));
  const [pet, setPet] = useState<PetForm>(() => (env.demoMode ? { ...DEMO_PET } : { ...EMPTY_PET }));
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);

  const [session, setSession] = useState<ProfileIssueStartResp | null>(null);
  const [status, setStatus] = useState<ProfileIssueStatusResp | null>(null);
  const [expired, setExpired] = useState(false);

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
    try {
      const resp = await api.startProfileIssue(buildBody());
      setSession(resp);
      setStatus(null);
      setExpired(false);
      toast({ title: "Issuance started", description: `Dog tag ${resp.dogTagId} allocated.`, variant: "success" });
    } catch (err) {
      toast({ title: "Start failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  function restart() {
    setSession(null);
    setStatus(null);
    setExpired(false);
  }

  const bound = status?.status === "bound";

  // Poll the session status every ~2s until it binds or the token expires.
  const startedAtRef = useRef<number>(0);
  useEffect(() => {
    if (!session || bound || expired) return;
    startedAtRef.current = Date.now();
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;

    const tick = async () => {
      if (cancelled) return;
      if (Date.now() - startedAtRef.current > TOKEN_TTL_MS) {
        setExpired(true);
        return;
      }
      try {
        const s = await api.profileIssueStatus(session.sessionId);
        if (cancelled) return;
        setStatus(s);
        if (s.status === "bound") return; // effect re-runs; bound short-circuits at the top
      } catch {
        /* transient — keep polling until expiry */
      }
      if (!cancelled) timer = setTimeout(tick, POLL_MS);
    };
    timer = setTimeout(tick, POLL_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [session, bound, expired, api]);

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

  if (session) {
    return (
      <Card>
        <CardHeader>
          <CardTitle>Owner scans to receive their dog tag</CardTitle>
          <CardDescription>
            Dog tag <Badge variant="neutral">{session.dogTagId}</Badge> allocated — pending device bind.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          {expired ? (
            <div className="space-y-3 text-center">
              <p className="text-sm text-danger">
                The QR token expired before the owner's device bound the dog tag (180s limit).
              </p>
              <Button onClick={restart}>Start over</Button>
            </div>
          ) : (
            <div className="flex flex-col items-center gap-3">
              <QrCode value={session.qr} caption={session.qr} />
              <p className="max-w-sm text-center text-sm text-muted">
                Owner scans this in the DogTag app to receive their dog tag. Waiting for the device to
                bind…
              </p>
              <Badge variant="neutral">Status: {status?.status ?? "pending"}</Badge>
              <Button variant="ghost" size="sm" onClick={restart}>
                Cancel / start over
              </Button>
            </div>
          )}
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardHeader className="flex flex-row items-start justify-between gap-4">
        <div>
          <CardTitle>Issue a dog tag</CardTitle>
          <CardDescription>
            Signing mode: <Badge variant="neutral">{signingMode}</Badge> — capture the owner identity +
            pet profile, then the owner's device binds the dog tag on-chain.
          </CardDescription>
        </div>
        {env.demoMode && (
          <Button type="button" variant="outline" size="sm" onClick={fillDemo}>
            <Sparkles className="h-4 w-4" /> Fill demo data
          </Button>
        )}
      </CardHeader>
      <CardContent>
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

          <Button type="submit" loading={busy}>
            Start issuance
          </Button>
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

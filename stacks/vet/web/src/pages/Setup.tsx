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
  DEMO_ADMIN_PASSWORD,
  DEMO_WHITELIST_APPLY_VET,
  type AccountInfo,
  type GenesisStartResp,
} from "@dogtag/ui";
import { Check, Copy, KeyRound, ShieldAlert } from "lucide-react";
import { useEffect, useState, type FormEvent } from "react";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";

type Step = "admin" | "genesis" | "confirm" | "unlock" | "accounts" | "apply" | "dns" | "done";

// Testnet demo: a fixed passphrase prefilled into genesis-confirm + unlock so you type nothing.
const DEMO_PASSPHRASE = "demo-pass-0000";

export function Setup() {
  const { api, adminToken, setAdminToken, setUnlocked, setSignerAddress } = useApp();
  const { toast } = useToast();
  const [step, setStep] = useState<Step>(adminToken ? "genesis" : "admin");

  // Stale-session recovery: if the admin (custody) token is cleared (e.g. a 401 after a backend
  // restart), drop back to the Custody admin login instead of stranding the user mid-wizard.
  useEffect(() => {
    if (!adminToken && step !== "admin") setStep("admin");
  }, [adminToken, step]);

  return (
    <div className="space-y-6">
      <SetupProgress step={step} />
      {step === "admin" && (
        <AdminLogin
          onDone={(tok) => {
            setAdminToken(tok);
            setStep("genesis");
          }}
          login={api.adminLogin}
          toast={toast}
        />
      )}
      {step === "genesis" && <GenesisStart onNext={() => setStep("confirm")} />}
      {step === "confirm" && (
        <GenesisConfirm
          onNext={(addr) => {
            if (addr) setSignerAddress(addr);
            setStep("unlock");
          }}
        />
      )}
      {step === "unlock" && (
        <Unlock
          onNext={(addr) => {
            setUnlocked(true);
            if (addr) setSignerAddress(addr);
            setStep("accounts");
          }}
        />
      )}
      {step === "accounts" && <DeriveAccounts onNext={() => setStep("apply")} />}
      {step === "apply" && <ApplyWhitelist onNext={() => setStep("dns")} />}
      {step === "dns" && <DnsInstructions onDone={() => setStep("done")} />}
      {step === "done" && (
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Check className="h-5 w-5 text-success" /> Setup complete
            </CardTitle>
            <CardDescription>
              Custody is initialized and unlocked. You can now issue credentials.
            </CardDescription>
          </CardHeader>
        </Card>
      )}
    </div>
  );
}

const STEPS: { key: Step; label: string }[] = [
  { key: "admin", label: "Admin" },
  { key: "genesis", label: "Genesis" },
  { key: "confirm", label: "Confirm" },
  { key: "unlock", label: "Unlock" },
  { key: "accounts", label: "Accounts" },
  { key: "apply", label: "Whitelist" },
  { key: "dns", label: "DNS" },
];

function SetupProgress({ step }: { step: Step }) {
  const idx = STEPS.findIndex((s) => s.key === step);
  return (
    <div className="flex flex-wrap items-center gap-2">
      {STEPS.map((s, i) => (
        <Badge key={s.key} variant={i < idx ? "success" : i === idx ? "default" : "neutral"}>
          {i + 1}. {s.label}
        </Badge>
      ))}
    </div>
  );
}

type Toast = ReturnType<typeof useToast>["toast"];

function AdminLogin({
  onDone,
  login,
  toast,
}: {
  onDone: (token: string) => void;
  login: (pw: string) => Promise<{ token: string }>;
  toast: Toast;
}) {
  // Testnet demo: prefill the admin password so the operator just clicks Continue.
  const [pw, setPw] = useState(env.demoMode ? DEMO_ADMIN_PASSWORD : "");
  const [busy, setBusy] = useState(false);
  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const r = await login(pw);
      onDone(r.token);
    } catch (err) {
      toast({ title: "Admin login failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <ShieldAlert className="h-5 w-5 text-primary" /> Custody admin login
        </CardTitle>
        <CardDescription>
          The custody (admin) session is separate from the operator session and gates genesis.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={submit} className="flex max-w-sm flex-col gap-3">
          <Label htmlFor="admin-pw" required>
            Admin password
          </Label>
          <Input id="admin-pw" type="password" value={pw} onChange={(e) => setPw(e.target.value)} required />
          {env.demoMode && (
            <p className="text-xs text-muted">Demo default prefilled — just click Continue.</p>
          )}
          <Button type="submit" loading={busy}>
            Continue
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}

function GenesisStart({ onNext }: { onNext: () => void }) {
  const { api } = useApp();
  const { toast } = useToast();
  const [data, setData] = useState<GenesisStartResp | null>(null);
  const [busy, setBusy] = useState(false);
  const [acknowledged, setAcknowledged] = useState(false);

  async function start() {
    setBusy(true);
    try {
      const r = await api.genesisStart();
      setData(r);
      sessionStorage.setItem("vet.challengeIndices", JSON.stringify(r.challengeIndices));
      // Testnet demo only: stash the words so the Confirm step's "Fill demo data" can re-enter
      // the challenge words for you. (NEVER do this in production — the seed must stay offline.)
      if (env.demoMode) {
        sessionStorage.setItem("vet.demoWords", JSON.stringify(r.words));
      }
    } catch (err) {
      toast({ title: "Genesis failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <KeyRound className="h-5 w-5 text-primary" /> Generate seed phrase
        </CardTitle>
        <CardDescription>
          Write down these 24 words and store them offline. They are shown only once.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        {!data ? (
          <Button onClick={start} loading={busy}>
            Generate 24-word seed
          </Button>
        ) : (
          <>
            <div className="grid grid-cols-2 gap-2 rounded-lg border border-border bg-surface-muted p-4 sm:grid-cols-3 md:grid-cols-4">
              {data.words.map((w, i) => (
                <div key={i} className="flex items-center gap-2 text-sm">
                  <span className="w-6 text-right text-muted">{i + 1}.</span>
                  <span className="font-mono">{w}</span>
                </div>
              ))}
            </div>
            <p className="text-sm text-muted">
              You will be asked to re-enter the words at positions:{" "}
              <span className="font-medium text-onSurface">
                {data.challengeIndices.map((i) => i + 1).join(", ")}
              </span>
            </p>
            <label className="flex items-center gap-2 text-sm">
              <input
                type="checkbox"
                checked={acknowledged}
                onChange={(e) => setAcknowledged(e.target.checked)}
              />
              I have written down all 24 words.
            </label>
            <Button onClick={onNext} disabled={!acknowledged}>
              Continue to confirmation
            </Button>
          </>
        )}
      </CardContent>
    </Card>
  );
}

function GenesisConfirm({ onNext }: { onNext: (address?: string) => void }) {
  const { api } = useApp();
  const { toast } = useToast();
  const indices: number[] = JSON.parse(sessionStorage.getItem("vet.challengeIndices") ?? "[]");
  // Testnet demo: auto-fill the challenge words from the just-shown seed + a default passphrase,
  // so you type NOTHING — just click "Confirm & encrypt". (Prod never stashes the seed.)
  const demoWords: string[] = env.demoMode
    ? JSON.parse(sessionStorage.getItem("vet.demoWords") ?? "[]")
    : [];
  const [words, setWords] = useState<string[]>(() => indices.map((idx) => demoWords[idx] ?? ""));
  const [passphrase, setPassphrase] = useState(env.demoMode ? DEMO_PASSPHRASE : "");
  const [busy, setBusy] = useState(false);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const r = await api.genesisConfirm({ words: words.map((w) => w.trim()), passphrase });
      toast({ title: "Custody initialized", description: r.address, variant: "success" });
      sessionStorage.removeItem("vet.challengeIndices");
      sessionStorage.removeItem("vet.demoWords");
      onNext(r.address);
    } catch (err) {
      toast({ title: "Confirmation failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Confirm seed + set passphrase</CardTitle>
        <CardDescription>
          Re-enter the challenge words and choose a passphrase that encrypts the seed at rest.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={submit} className="space-y-4">
          <div className="grid gap-3 sm:grid-cols-3">
            {indices.map((idx, i) => (
              <div key={idx} className="space-y-1.5">
                <Label htmlFor={`w-${idx}`} required>
                  Word #{idx + 1}
                </Label>
                <Input
                  id={`w-${idx}`}
                  value={words[i] ?? ""}
                  onChange={(e) =>
                    setWords((prev) => prev.map((w, j) => (j === i ? e.target.value : w)))
                  }
                  required
                />
              </div>
            ))}
          </div>
          <div className="max-w-sm space-y-1.5">
            <Label htmlFor="passphrase" required>
              Encryption passphrase
            </Label>
            <Input
              id="passphrase"
              type="password"
              value={passphrase}
              onChange={(e) => setPassphrase(e.target.value)}
              required
            />
          </div>
          <Button type="submit" loading={busy}>
            Confirm & encrypt
          </Button>
        </form>
      </CardContent>
    </Card>
  );
}

function Unlock({ onNext }: { onNext: (address?: string) => void }) {
  const { api } = useApp();
  const { toast } = useToast();
  const [passphrase, setPassphrase] = useState(env.demoMode ? DEMO_PASSPHRASE : "");
  const [busy, setBusy] = useState(false);
  const [accounts, setAccounts] = useState<AccountInfo[] | null>(null);

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const r = await api.unlock({ passphrase });
      setAccounts(r.accounts);
      toast({ title: "Unlocked", variant: "success" });
    } catch (err) {
      toast({ title: "Unlock failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Unlock custody</CardTitle>
        <CardDescription>Decrypt the seed and wire the backend signers into the chain client.</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <form onSubmit={submit} className="flex max-w-sm flex-col gap-3">
          <Label htmlFor="unlock-pw" required>
            Passphrase
          </Label>
          <Input
            id="unlock-pw"
            type="password"
            value={passphrase}
            onChange={(e) => setPassphrase(e.target.value)}
            required
          />
          <Button type="submit" loading={busy}>
            Unlock
          </Button>
        </form>
        {accounts && (
          <>
            <div className="space-y-1">
              {accounts.map((a) => (
                <div key={a.index} className="flex items-center gap-2 text-sm">
                  <Badge variant="neutral">#{a.index}</Badge>
                  <span className="font-mono">{a.address}</span>
                  <span className="text-muted">{a.label}</span>
                </div>
              ))}
            </div>
            <Button onClick={() => onNext(accounts[0]?.address)}>Continue</Button>
          </>
        )}
      </CardContent>
    </Card>
  );
}

function DeriveAccounts({ onNext }: { onNext: () => void }) {
  const { api } = useApp();
  const { toast } = useToast();
  const [label, setLabel] = useState("");
  const [busy, setBusy] = useState(false);
  const [derived, setDerived] = useState<{ index: number; address: string }[]>([]);

  async function add(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const r = await api.addAccount({ label: label || `account${derived.length + 1}` });
      setDerived((prev) => [...prev, r]);
      setLabel("");
      toast({ title: "Account derived", description: r.address, variant: "success" });
    } catch (err) {
      toast({ title: "Derive failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Derive accounts</CardTitle>
        <CardDescription>Optionally derive additional signer addresses (account 0 already exists).</CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <form onSubmit={add} className="flex items-end gap-2">
          <div className="flex-1 space-y-1.5">
            <Label htmlFor="acct-label">Label</Label>
            <Input id="acct-label" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="e.g. front-desk" />
          </div>
          <Button type="submit" variant="outline" loading={busy}>
            Derive
          </Button>
        </form>
        {derived.map((d) => (
          <div key={d.index} className="flex items-center gap-2 text-sm">
            <Badge variant="neutral">#{d.index}</Badge>
            <span className="font-mono">{d.address}</span>
          </div>
        ))}
        <Button onClick={onNext}>Continue to whitelist application</Button>
      </CardContent>
    </Card>
  );
}

function ApplyWhitelist({ onNext }: { onNext: () => void }) {
  const { api, signerAddress } = useApp();
  const { toast } = useToast();
  // Testnet demo: prefill every field by default (signer auto-carried) so you just click Submit.
  // Production (demo off): demo-ish fields start empty; recordTypes + signer auto-carry are not
  // demo data so they're kept, and documentStore falls back to the configured issuer address.
  const [form, setForm] = useState(() => ({
    issuerEntityId: env.demoMode ? "seaport-vet" : "",
    address: signerAddress ?? "",
    recordTypes: "VACCINATION",
    domain: env.demoMode ? "testvet.roax.net" : "",
    documentStore: env.demoMode
      ? env.dogtagIssuerAddr || "0x5c703910111f942EE0f47E02214291b5274cDb53"
      : env.dogtagIssuerAddr || "",
    usdaNan: env.demoMode ? "123456" : "",
    licenseNumber: env.demoMode ? "VET-2024-0001" : "",
    licenseJurisdiction: env.demoMode ? "CA" : "",
    licenseExpiry: env.demoMode ? "2027-12-31" : "",
  }));
  const [busy, setBusy] = useState(false);
  const [applicationId, setApplicationId] = useState<string | null>(null);

  // Keep the signer address synced if it lands after this form mounts.
  useEffect(() => {
    if (signerAddress) setForm((p) => (p.address ? p : { ...p, address: signerAddress }));
  }, [signerAddress]);

  function upd(k: keyof typeof form, v: string) {
    setForm((p) => ({ ...p, [k]: v }));
  }

  // Testnet demo: fill everything. The signer address is auto-carried from genesis (kept as-is here).
  function fillDemo() {
    setForm((p) => ({
      ...p,
      issuerEntityId: DEMO_WHITELIST_APPLY_VET.issuerEntityId,
      address: p.address || signerAddress || "",
      recordTypes: DEMO_WHITELIST_APPLY_VET.recordTypes,
      domain: DEMO_WHITELIST_APPLY_VET.domain,
      documentStore: env.dogtagIssuerAddr || DEMO_WHITELIST_APPLY_VET.documentStore,
      usdaNan: DEMO_WHITELIST_APPLY_VET.usdaNan,
      licenseNumber: DEMO_WHITELIST_APPLY_VET.licenseNumber,
      licenseJurisdiction: DEMO_WHITELIST_APPLY_VET.licenseJurisdiction,
      licenseExpiry: DEMO_WHITELIST_APPLY_VET.licenseExpiry,
    }));
    toast({ title: "Demo data filled", description: "Review and Submit application.", variant: "success" });
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const r = await api.applyForWhitelist({
        issuerEntityId: form.issuerEntityId,
        addresses: [form.address],
        recordTypes: form.recordTypes.split(",").map((s) => s.trim()).filter(Boolean),
        domain: form.domain,
        documentStore: form.documentStore,
        usdaNan: form.usdaNan || undefined,
        license: form.licenseNumber
          ? {
              number: form.licenseNumber,
              jurisdiction: form.licenseJurisdiction,
              expiry: form.licenseExpiry,
            }
          : undefined,
      });
      setApplicationId(r.applicationId);
      toast({ title: "Application submitted", description: `Status: ${r.status}`, variant: "success" });
    } catch (err) {
      toast({ title: "Apply failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Card>
      <CardHeader>
        <CardTitle>Apply for whitelist</CardTitle>
        <CardDescription>
          Submit your USDA accreditation / license to the central registry. An admin approves it
          out-of-band (on-chain whitelist). The signer address is auto-carried from genesis.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <form onSubmit={submit} className="grid gap-4 sm:grid-cols-2">
          <Field label="Issuer entity id" required value={form.issuerEntityId} onChange={(v) => upd("issuerEntityId", v)} />
          <Field label="Signer address (auto-filled)" required value={form.address} onChange={(v) => upd("address", v)} placeholder="0x…" />
          <Field label="Record types (comma-separated)" required value={form.recordTypes} onChange={(v) => upd("recordTypes", v)} />
          <Field label="Domain" required value={form.domain} onChange={(v) => upd("domain", v)} placeholder="clinic.example.com" />
          <Field label="Document store address" required value={form.documentStore} onChange={(v) => upd("documentStore", v)} placeholder="0x…" />
          <Field label="USDA NAN (6 digits)" value={form.usdaNan} onChange={(v) => upd("usdaNan", v)} placeholder="123456" />
          <Field label="License number" value={form.licenseNumber} onChange={(v) => upd("licenseNumber", v)} />
          <Field label="License jurisdiction" value={form.licenseJurisdiction} onChange={(v) => upd("licenseJurisdiction", v)} />
          <Field label="License expiry" value={form.licenseExpiry} onChange={(v) => upd("licenseExpiry", v)} placeholder="YYYY-MM-DD" />
          <div className="sm:col-span-2 flex items-center gap-3">
            {env.demoMode && (
              <Button type="button" variant="ghost" onClick={fillDemo}>
                Fill demo data
              </Button>
            )}
            <Button type="submit" loading={busy}>
              Submit application
            </Button>
            {applicationId && (
              <>
                <Badge variant="success">pending</Badge>
                <Button type="button" variant="ghost" onClick={onNext}>
                  Continue
                </Button>
              </>
            )}
          </div>
        </form>
      </CardContent>
    </Card>
  );
}

function Field({
  label,
  value,
  onChange,
  required,
  placeholder,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  required?: boolean;
  placeholder?: string;
}) {
  return (
    <div className="space-y-1.5">
      <Label required={required}>{label}</Label>
      <Input value={value} onChange={(e) => onChange(e.target.value)} required={required} placeholder={placeholder} />
    </div>
  );
}

function DnsInstructions({ onDone }: { onDone: () => void }) {
  const [domain, setDomain] = useState("");
  const [copied, setCopied] = useState(false);
  const txt = `dogtag-issuer=${env.deploymentUrl}`;

  return (
    <Card>
      <CardHeader>
        <CardTitle>DNS-TXT verification</CardTitle>
        <CardDescription>
          Add this TXT record to your domain so verifiers can resolve your issuer identity.
        </CardDescription>
      </CardHeader>
      <CardContent className="space-y-4">
        <div className="max-w-sm space-y-1.5">
          <Label>Your domain</Label>
          <Input value={domain} onChange={(e) => setDomain(e.target.value)} placeholder="clinic.example.com" />
        </div>
        <div className="rounded-lg border border-border bg-surface-muted p-4 font-mono text-sm">
          <div>
            <span className="text-muted">Host:</span> _dogtag.{domain || "<your-domain>"}
          </div>
          <div>
            <span className="text-muted">Type:</span> TXT
          </div>
          <div className="flex items-center justify-between gap-2">
            <span>
              <span className="text-muted">Value:</span> {txt}
            </span>
            <Button
              type="button"
              size="sm"
              variant="ghost"
              onClick={() => {
                void navigator.clipboard?.writeText(txt);
                setCopied(true);
              }}
            >
              {copied ? <Check className="h-4 w-4" /> : <Copy className="h-4 w-4" />}
            </Button>
          </div>
        </div>
        <Button onClick={onDone}>Done</Button>
      </CardContent>
    </Card>
  );
}

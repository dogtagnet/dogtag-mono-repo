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
  Spinner,
  explorerTxUrl,
  useToast,
  DEMO_BUSINESS_GROOMER,
  DEMO_BUSINESS_VET,
  DEMO_ISSUER_APPLICATION_GROOMER,
  DEMO_ISSUER_APPLICATION_VET,
  type DemoBusiness,
  type DemoIssuerApplication,
  type RegisterBusinessResp,
} from "@dogtag/ui";
import {
  ArrowRight,
  Building2,
  CheckCircle2,
  Copy,
  ListChecks,
  RotateCcw,
  ShieldCheck,
  Sparkles,
} from "lucide-react";
import { useState, type FormEvent } from "react";
import { useApp } from "../app/AppContext";

/**
 * Add → approve issuer (end-to-end) wizard. A non-technical operator follows three linear steps:
 *   1. Register a business (POST /v1/businesses) → the HMAC key is issued ONCE.
 *   2. Submit an issuer application (POST /v1/issuer-applications) — multi-address × multi-recordType.
 *   3. Approve (POST /v1/issuer-applications/{id}/approve) → the central backend calls on-chain
 *      whitelistFor per (address, recordType); the resulting tx hashes are surfaced.
 * After step 3 the business can issue credentials. Each step is gated on the previous completing.
 */
type Preset = "vet" | "groomer";

export function Wizard() {
  const { central } = useApp();
  const { toast } = useToast();

  const [preset, setPreset] = useState<Preset>("vet");

  // step 1 — register business
  const [biz, setBiz] = useState<DemoBusiness>({ ...DEMO_BUSINESS_VET });
  const [bizResult, setBizResult] = useState<RegisterBusinessResp | null>(null);
  const [bizBusy, setBizBusy] = useState(false);

  // step 2 — submit application
  const [app, setApp] = useState<DemoIssuerApplication>({ ...DEMO_ISSUER_APPLICATION_VET });
  const [appId, setAppId] = useState<string | null>(null);
  const [appBusy, setAppBusy] = useState(false);

  // step 3 — approve
  const [whitelistTxs, setWhitelistTxs] = useState<string[] | null>(null);
  const [approveBusy, setApproveBusy] = useState(false);

  function applyPreset(p: Preset) {
    setPreset(p);
    setBiz({ ...(p === "vet" ? DEMO_BUSINESS_VET : DEMO_BUSINESS_GROOMER) });
    setApp({ ...(p === "vet" ? DEMO_ISSUER_APPLICATION_VET : DEMO_ISSUER_APPLICATION_GROOMER) });
  }

  async function registerBusiness(e: FormEvent) {
    e.preventDefault();
    setBizBusy(true);
    try {
      const r = await central.registerBusiness({
        type: biz.type,
        name: biz.name,
        lat: Number(biz.lat),
        lng: Number(biz.lng),
        services: biz.services.split(",").map((s) => s.trim()).filter(Boolean),
        apiBaseUrl: biz.apiBaseUrl,
        domain: biz.domain,
        documentStores: biz.documentStores.split(",").map((s) => s.trim()).filter(Boolean),
      });
      setBizResult(r);
      toast({ title: "Business registered", description: "HMAC key issued (save it now).", variant: "success" });
    } catch (err) {
      toast({ title: "Register failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBizBusy(false);
    }
  }

  async function submitApplication(e: FormEvent) {
    e.preventDefault();
    setAppBusy(true);
    try {
      const r = await central.createApplication({
        issuerEntityId: app.issuerEntityId,
        addresses: app.addresses.split(",").map((s) => s.trim()).filter(Boolean),
        recordTypes: app.recordTypes.split(",").map((s) => s.trim()).filter(Boolean),
        domain: app.domain,
        documentStore: app.documentStore,
        usdaNan: app.usdaNan.trim() || undefined,
      });
      setAppId(r.applicationId);
      toast({ title: "Application submitted", description: `pending — ${r.applicationId.slice(0, 8)}…`, variant: "success" });
    } catch (err) {
      toast({ title: "Submit failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setAppBusy(false);
    }
  }

  async function approve() {
    if (!appId) return;
    setApproveBusy(true);
    try {
      const r = await central.approveApplication(appId);
      setWhitelistTxs(r.whitelistTxs);
      toast({
        title: "Approved — whitelisted on-chain",
        description: `${r.whitelistTxs.length} whitelistFor tx(s) sent`,
        variant: "success",
      });
    } catch (err) {
      toast({ title: "Approve failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setApproveBusy(false);
    }
  }

  function restart() {
    setBizResult(null);
    setAppId(null);
    setWhitelistTxs(null);
    applyPreset(preset);
  }

  const step1Done = bizResult !== null;
  const step2Done = appId !== null;
  const step3Done = whitelistTxs !== null;

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader className="flex flex-row items-start justify-between gap-4">
          <div>
            <CardTitle>Onboard an issuer (end-to-end)</CardTitle>
            <CardDescription>
              1. Register business → 2. Submit issuer application → 3. Approve (whitelists on-chain) →
              done — this business can now issue credentials.
            </CardDescription>
          </div>
          <div className="flex gap-2">
            <Button
              type="button"
              size="sm"
              variant={preset === "vet" ? "primary" : "outline"}
              onClick={() => applyPreset("vet")}
            >
              <Sparkles className="h-4 w-4" /> Vet preset
            </Button>
            <Button
              type="button"
              size="sm"
              variant={preset === "groomer" ? "primary" : "outline"}
              onClick={() => applyPreset("groomer")}
            >
              <Sparkles className="h-4 w-4" /> Groomer preset
            </Button>
          </div>
        </CardHeader>
        <CardContent>
          <Stepper step1Done={step1Done} step2Done={step2Done} step3Done={step3Done} />
        </CardContent>
      </Card>

      {/* Step 1 — Register business */}
      <StepCard
        n={1}
        title="Register business"
        icon={<Building2 className="h-5 w-5 text-primary" />}
        done={step1Done}
      >
        {step1Done && bizResult ? (
          <div className="space-y-2 text-sm">
            <p className="text-sm text-success">Registered. Save the HMAC secret — shown only once.</p>
            <SecretRow label="Business ID" value={bizResult.businessId} />
            <SecretRow label="HMAC key ID" value={bizResult.hmacKeyId} />
            <SecretRow label="HMAC secret" value={bizResult.hmacSecret} mono />
          </div>
        ) : (
          <form onSubmit={registerBusiness} className="grid gap-3 sm:grid-cols-2">
            <Field label="Type" value={biz.type} onChange={(v) => setBiz({ ...biz, type: v })} required />
            <Field label="Name" value={biz.name} onChange={(v) => setBiz({ ...biz, name: v })} required />
            <Field label="Latitude" value={biz.lat} onChange={(v) => setBiz({ ...biz, lat: v })} required />
            <Field label="Longitude" value={biz.lng} onChange={(v) => setBiz({ ...biz, lng: v })} required />
            <Field label="API base URL" value={biz.apiBaseUrl} onChange={(v) => setBiz({ ...biz, apiBaseUrl: v })} required />
            <Field label="Domain" value={biz.domain} onChange={(v) => setBiz({ ...biz, domain: v })} required />
            <Field label="Services (comma)" value={biz.services} onChange={(v) => setBiz({ ...biz, services: v })} />
            <Field label="Document stores (comma)" value={biz.documentStores} onChange={(v) => setBiz({ ...biz, documentStores: v })} />
            <div className="sm:col-span-2">
              <Button type="submit" loading={bizBusy}>
                Register business <ArrowRight className="h-4 w-4" />
              </Button>
            </div>
          </form>
        )}
      </StepCard>

      {/* Step 2 — Submit application */}
      <StepCard
        n={2}
        title="Submit issuer application"
        icon={<ListChecks className="h-5 w-5 text-primary" />}
        done={step2Done}
        disabled={!step1Done}
      >
        {step2Done && appId ? (
          <div className="space-y-1 text-sm">
            <p className="text-success">Application submitted (pending review).</p>
            <SecretRow label="Application ID" value={appId} mono />
          </div>
        ) : (
          <form onSubmit={submitApplication} className="grid gap-3 sm:grid-cols-2">
            <Field label="Issuer entity id" value={app.issuerEntityId} onChange={(v) => setApp({ ...app, issuerEntityId: v })} required />
            <Field label="Domain" value={app.domain} onChange={(v) => setApp({ ...app, domain: v })} required />
            <Field label="Addresses (comma)" value={app.addresses} onChange={(v) => setApp({ ...app, addresses: v })} required />
            <Field label="Record types (comma)" value={app.recordTypes} onChange={(v) => setApp({ ...app, recordTypes: v })} required />
            <Field label="Document store" value={app.documentStore} onChange={(v) => setApp({ ...app, documentStore: v })} required />
            <Field label="USDA NAN (optional)" value={app.usdaNan} onChange={(v) => setApp({ ...app, usdaNan: v })} placeholder="6 digits" />
            <div className="sm:col-span-2">
              <Button type="submit" loading={appBusy} disabled={!step1Done}>
                Submit application <ArrowRight className="h-4 w-4" />
              </Button>
            </div>
          </form>
        )}
      </StepCard>

      {/* Step 3 — Approve */}
      <StepCard
        n={3}
        title="Approve (whitelists on-chain)"
        icon={<ShieldCheck className="h-5 w-5 text-primary" />}
        done={step3Done}
        disabled={!step2Done}
      >
        {step3Done && whitelistTxs ? (
          <div className="space-y-3">
            <div className="flex items-center gap-2">
              <CheckCircle2 className="h-6 w-6 text-success" />
              <span className="font-medium">
                Done — this business can now issue credentials.
              </span>
            </div>
            <p className="text-sm text-muted">
              {whitelistTxs.length} on-chain <code>whitelistFor(keccak256(recordType), address)</code>{" "}
              transaction(s) sent:
            </p>
            <div className="flex flex-col gap-1">
              {whitelistTxs.length === 0 ? (
                <span className="text-sm text-muted">No tx hashes returned (already whitelisted?).</span>
              ) : (
                whitelistTxs.map((tx) => (
                  <a
                    key={tx}
                    href={explorerTxUrl(tx)}
                    target="_blank"
                    rel="noreferrer"
                    className="break-all font-mono text-sm text-primary hover:underline"
                  >
                    {tx}
                  </a>
                ))
              )}
            </div>
            <Button variant="outline" size="sm" onClick={restart}>
              <RotateCcw className="h-4 w-4" /> Onboard another
            </Button>
          </div>
        ) : (
          <div className="space-y-3">
            <p className="text-sm text-muted">
              Approving runs DNS-TXT + accreditation checks then calls on-chain whitelistFor for every
              (address × recordType) pair. The resulting tx hashes appear here.
            </p>
            <Button onClick={approve} loading={approveBusy} disabled={!step2Done}>
              {approveBusy ? <Spinner className="h-4 w-4" /> : <ShieldCheck className="h-4 w-4" />} Approve &amp; whitelist
            </Button>
          </div>
        )}
      </StepCard>
    </div>
  );
}

function Stepper({
  step1Done,
  step2Done,
  step3Done,
}: {
  step1Done: boolean;
  step2Done: boolean;
  step3Done: boolean;
}) {
  const steps = [
    { label: "Register business", done: step1Done },
    { label: "Submit application", done: step2Done },
    { label: "Approve (on-chain)", done: step3Done },
  ];
  return (
    <ol className="flex flex-wrap items-center gap-2">
      {steps.map((s, i) => (
        <li key={s.label} className="flex items-center gap-2">
          <Badge variant={s.done ? "success" : "neutral"}>
            {s.done ? <CheckCircle2 className="h-3 w-3" /> : <span className="font-mono">{i + 1}</span>} {s.label}
          </Badge>
          {i < steps.length - 1 && <ArrowRight className="h-4 w-4 text-muted" />}
        </li>
      ))}
    </ol>
  );
}

function StepCard({
  n,
  title,
  icon,
  done,
  disabled,
  children,
}: {
  n: number;
  title: string;
  icon: React.ReactNode;
  done?: boolean;
  disabled?: boolean;
  children: React.ReactNode;
}) {
  return (
    <Card className={disabled ? "opacity-60" : undefined}>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <span className="flex h-6 w-6 items-center justify-center rounded-full bg-primary/10 text-xs font-bold text-primary">
            {n}
          </span>
          {icon}
          {title}
          {done && <CheckCircle2 className="h-4 w-4 text-success" />}
        </CardTitle>
      </CardHeader>
      <CardContent>{disabled ? <p className="text-sm text-muted">Complete the previous step first.</p> : children}</CardContent>
    </Card>
  );
}

function SecretRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-center justify-between gap-2 rounded-md border border-border px-3 py-2">
      <div className="min-w-0">
        <div className="text-xs text-muted">{label}</div>
        <div className={`truncate ${mono ? "font-mono" : ""}`}>{value}</div>
      </div>
      <Button size="sm" variant="ghost" onClick={() => void navigator.clipboard?.writeText(value)}>
        <Copy className="h-4 w-4" />
      </Button>
    </div>
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

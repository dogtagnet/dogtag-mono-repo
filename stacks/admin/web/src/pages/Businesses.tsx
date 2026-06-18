import {
  Badge,
  Button,
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Spinner,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  useToast,
  DEMO_BUSINESS_GROOMER,
  DEMO_BUSINESS_VET,
  type CentralBusiness,
} from "@dogtag/ui";
import { Building2, Copy, MapPin, Plus, Sparkles } from "lucide-react";
import { useCallback, useEffect, useState, type FormEvent } from "react";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";

/**
 * Business registry (impl §5.3 / §4.2). Lists businesses from GET /v1/businesses (public discovery)
 * and registers new ones via POST /v1/businesses (admin-gated; the HMAC secret is returned ONCE).
 * Geo coords are shown in the table; a map is optional and skipped here to avoid a heavy dep.
 */
export function Businesses() {
  const { central } = useApp();
  const { toast } = useToast();
  const [rows, setRows] = useState<CentralBusiness[] | null>(null);
  const [open, setOpen] = useState(false);
  const [secret, setSecret] = useState<{ businessId: string; hmacKeyId: string; hmacSecret: string } | null>(null);

  const load = useCallback(async () => {
    try {
      const r = await central.listBusinesses();
      setRows(r.businesses);
    } catch (err) {
      toast({ title: "Failed to load businesses", description: (err as Error).message, variant: "danger" });
      setRows([]);
    }
  }, [central, toast]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="space-y-6">
      <Card>
        <CardHeader className="flex flex-row items-start justify-between gap-4">
          <div>
            <CardTitle className="flex items-center gap-2">
              <Building2 className="h-5 w-5 text-primary" /> Business registry
            </CardTitle>
            <CardDescription>
              Registered vets, groomers and other businesses (discovery + appointment routing).
            </CardDescription>
          </div>
          <Button onClick={() => setOpen(true)}>
            <Plus className="h-4 w-4" /> Register
          </Button>
        </CardHeader>
        <CardContent>
          {rows === null ? (
            <div className="flex justify-center py-8">
              <Spinner className="h-5 w-5 text-muted" />
            </div>
          ) : rows.length === 0 ? (
            <p className="py-8 text-center text-sm text-muted">No businesses registered yet.</p>
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Type</TableHead>
                  <TableHead>Domain</TableHead>
                  <TableHead>Geo</TableHead>
                  <TableHead>Services</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((b) => (
                  <TableRow key={b.businessId}>
                    <TableCell className="font-medium">{b.name}</TableCell>
                    <TableCell>
                      <Badge variant="neutral">{b.type}</Badge>
                    </TableCell>
                    <TableCell className="font-mono text-xs">{b.domain}</TableCell>
                    <TableCell className="whitespace-nowrap text-xs text-muted">
                      <span className="inline-flex items-center gap-1">
                        <MapPin className="h-3 w-3" />
                        {b.geo.lat.toFixed(4)}, {b.geo.lng.toFixed(4)}
                      </span>
                    </TableCell>
                    <TableCell className="text-xs text-muted">{b.services.join(", ") || "—"}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
          <p className="mt-4 text-xs text-muted">
            Note: a geographic map view is optional and intentionally omitted (no map dependency).
            Latitude/longitude are shown in the table; discovery filters (<code>type</code>,{" "}
            <code>near</code>, <code>radius</code>) are supported by the backend.
          </p>
        </CardContent>
      </Card>

      <RegisterDialog
        open={open}
        onOpenChange={setOpen}
        onRegistered={(r) => {
          setSecret(r);
          void load();
        }}
      />

      <Dialog open={secret !== null} onOpenChange={(o) => !o && setSecret(null)}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Business registered</DialogTitle>
            <DialogDescription>
              Save the HMAC secret now — it is shown only once (like an API key).
            </DialogDescription>
          </DialogHeader>
          {secret && (
            <div className="space-y-2 text-sm">
              <SecretRow label="Business ID" value={secret.businessId} />
              <SecretRow label="HMAC key ID" value={secret.hmacKeyId} />
              <SecretRow label="HMAC secret" value={secret.hmacSecret} mono />
            </div>
          )}
        </DialogContent>
      </Dialog>
    </div>
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

function RegisterDialog({
  open,
  onOpenChange,
  onRegistered,
}: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  onRegistered: (r: { businessId: string; hmacKeyId: string; hmacSecret: string }) => void;
}) {
  const { central } = useApp();
  const { toast } = useToast();
  const [form, setForm] = useState({
    type: "groomer",
    name: "",
    lat: "",
    lng: "",
    services: "",
    apiBaseUrl: "",
    domain: "",
    documentStores: "",
  });
  const [busy, setBusy] = useState(false);

  function upd(k: keyof typeof form, v: string) {
    setForm((p) => ({ ...p, [k]: v }));
  }

  async function submit(e: FormEvent) {
    e.preventDefault();
    setBusy(true);
    try {
      const r = await central.registerBusiness({
        type: form.type,
        name: form.name,
        lat: Number(form.lat),
        lng: Number(form.lng),
        services: form.services.split(",").map((s) => s.trim()).filter(Boolean),
        apiBaseUrl: form.apiBaseUrl,
        domain: form.domain,
        documentStores: form.documentStores.split(",").map((s) => s.trim()).filter(Boolean),
      });
      onRegistered(r);
      onOpenChange(false);
      toast({ title: "Business registered", variant: "success" });
    } catch (err) {
      toast({ title: "Register failed", description: (err as Error).message, variant: "danger" });
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-lg">
        <DialogHeader>
          <DialogTitle>Register a business</DialogTitle>
          <DialogDescription>Admin-gated. Non-personal fields only.</DialogDescription>
        </DialogHeader>
        {env.demoMode && (
          <div className="flex flex-wrap gap-2">
            <Button type="button" variant="outline" size="sm" onClick={() => setForm({ ...DEMO_BUSINESS_VET })}>
              <Sparkles className="h-4 w-4" /> Demo (vet)
            </Button>
            <Button type="button" variant="outline" size="sm" onClick={() => setForm({ ...DEMO_BUSINESS_GROOMER })}>
              <Sparkles className="h-4 w-4" /> Demo (groomer)
            </Button>
          </div>
        )}
        <form onSubmit={submit} className="grid gap-3 sm:grid-cols-2">
          <Field label="Type" value={form.type} onChange={(v) => upd("type", v)} placeholder="groomer / vet" required />
          <Field label="Name" value={form.name} onChange={(v) => upd("name", v)} required />
          <Field label="Latitude" value={form.lat} onChange={(v) => upd("lat", v)} placeholder="37.7749" required />
          <Field label="Longitude" value={form.lng} onChange={(v) => upd("lng", v)} placeholder="-122.4194" required />
          <Field label="API base URL" value={form.apiBaseUrl} onChange={(v) => upd("apiBaseUrl", v)} placeholder="https://…" required />
          <Field label="Domain" value={form.domain} onChange={(v) => upd("domain", v)} placeholder="shop.example.com" required />
          <Field label="Services (comma)" value={form.services} onChange={(v) => upd("services", v)} placeholder="grooming, boarding" />
          <Field label="Document stores (comma)" value={form.documentStores} onChange={(v) => upd("documentStores", v)} placeholder="0x…" />
          <div className="sm:col-span-2">
            <Button type="submit" loading={busy}>
              Register
            </Button>
          </div>
        </form>
      </DialogContent>
    </Dialog>
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

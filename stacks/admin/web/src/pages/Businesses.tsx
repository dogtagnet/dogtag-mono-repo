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
  blankContactFields,
  contactRequestFields,
  locationRequestFields,
  parseLocationInput,
  DEMO_BUSINESS_CONTACT_ONLY,
  DEMO_BUSINESS_GROOMER,
  DEMO_BUSINESS_VET,
  PROVIDER_CONTACT_CHANNELS,
  type BusinessContact,
  type BusinessLocationReviewResp,
  type CentralBusiness,
  type ContactChannelRecord,
} from "@dogtag/ui";
import { AlertTriangle, Building2, Copy, MapPin, Plus, Sparkles } from "lucide-react";
import { useCallback, useEffect, useState, type FormEvent } from "react";
import { useApp } from "../app/AppContext";
import { env } from "../lib/env";

/**
 * How each channel is labelled here. The KEYS come from the shared list, so a channel added there
 * is a compile error in this record rather than a field this form silently never offers.
 */
const CONTACT_PRESENTATION: ContactChannelRecord<{ label: string; placeholder: string }> = {
  phone: { label: "Phone", placeholder: "+65 6123 4567" },
  whatsapp: { label: "WhatsApp", placeholder: "+65 9123 4567" },
  telegram: { label: "Telegram", placeholder: "@handle" },
  email: { label: "Business email", placeholder: "hello@shop.example" },
  website: { label: "Website", placeholder: "https://shop.example" },
};

/** The channels a provider may publish, in the order the table and form offer them. */
const CONTACT_FIELDS = PROVIDER_CONTACT_CHANNELS.map((key) => ({
  key,
  ...CONTACT_PRESENTATION[key],
}));

/** The published channels of one row, in listing order. */
function contactEntries(contact: BusinessContact | undefined): { label: string; value: string }[] {
  if (!contact) return [];
  return CONTACT_FIELDS.flatMap(({ key, label }) => {
    const value = contact[key];
    return typeof value === "string" && value.trim() ? [{ label, value: value.trim() }] : [];
  });
}

/**
 * Business registry (impl §5.3 / §4.2). Lists businesses from GET /v1/businesses (public discovery)
 * and registers new ones via POST /v1/businesses (admin-gated; the HMAC secret is returned ONCE).
 * There is deliberately no in-app map.
 *
 * LOCATION IS OPTIONAL, and that is the point of the Geo column's two states. A provider may publish
 * contact channels and no address; it is listed and contactable, and it is shown as "No location"
 * rather than as a coordinate. It previously had none: a blank location was stored as `0, 0`, which
 * is a legal point in the Gulf of Guinea, so it rendered here as a confident pin off the coast of
 * Ghana. Never substitute a placeholder coordinate for an absent one.
 */
/**
 * THREE states, and the initial one is never the failure one.
 *
 * "The review has not been read yet", "it was read and nothing needs an answer" and "it could not be
 * read" are three different claims. Collapsing the first two into one absent value is how a lapsed
 * session reads as all-clear on the page whose only signal is this banner; collapsing the first and
 * third is how a page that has issued no request yet announces that the check failed.
 */
type ReviewState =
  | { status: "checking" }
  | { status: "loaded"; review: BusinessLocationReviewResp }
  | { status: "failed" };

export function Businesses() {
  const { central } = useApp();
  const { toast } = useToast();
  const [rows, setRows] = useState<CentralBusiness[] | null>(null);
  const [review, setReview] = useState<ReviewState>({ status: "checking" });
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
    // Best-effort and deliberately separate: a review listing that cannot be read must not make the
    // registry itself look empty. Back to "checking" first, so a re-load after registering does not
    // keep presenting the previous answer as if it were current.
    setReview({ status: "checking" });
    try {
      setReview({ status: "loaded", review: await central.businessesLocationReview() });
    } catch {
      setReview({ status: "failed" });
    }
  }, [central, toast]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="space-y-6">
      {/* A read still in flight renders NEITHER banner: it has established nothing yet, so it may
          claim neither that rows need an answer nor that the check failed. */}
      {review.status === "failed" && <LocationReviewUnavailable />}
      {review.status === "loaded" && review.review.needsReview > 0 && (
        <LocationReviewBanner review={review.review} />
      )}
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
                  <TableHead>Contact</TableHead>
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
                      <GeoCell geo={b.geo} />
                    </TableCell>
                    <TableCell className="text-xs text-muted">
                      <ContactCell contact={b.contact} />
                    </TableCell>
                    <TableCell className="text-xs text-muted">{b.services.join(", ") || "—"}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          )}
          <p className="mt-4 text-xs text-muted">
            Note: this page fetches the full business set with no discovery filters and shows each
            latitude/longitude in the table. There is deliberately no in-app map. No position is ever
            sent to the server - <code>near</code> and <code>radius</code> are deprecated for that
            reason, and proximity filtering belongs on the device that needs it.
          </p>
          <p className="mt-2 text-xs text-muted">
            Location is optional. A provider with no address is listed and reached through its
            contact channels; it shows as <em>No location</em> rather than a coordinate, and it is
            never given a distance or a Directions destination.
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

/**
 * A coordinate, or the absence of one - never a placeholder.
 *
 * "No location" is rendered as its own neutral state rather than as `0.0000, 0.0000`, which is what
 * a blank field used to become and which reads as a real pin in the Gulf of Guinea.
 */
function GeoCell({ geo }: { geo: CentralBusiness["geo"] }) {
  if (!geo) return <span className="italic">No location</span>;
  return (
    <span className="inline-flex items-center gap-1">
      <MapPin className="h-3 w-3" />
      {geo.lat.toFixed(4)}, {geo.lng.toFixed(4)}
    </span>
  );
}

function ContactCell({ contact }: { contact: BusinessContact | undefined }) {
  const entries = contactEntries(contact);
  if (entries.length === 0) return <span className="italic">—</span>;
  return (
    <div className="space-y-0.5">
      {entries.map((e) => (
        <div key={e.label}>
          <span className="text-muted">{e.label}: </span>
          {e.value}
        </div>
      ))}
    </div>
  );
}

/**
 * The location review could not be read.
 *
 * Its own state rather than an absent banner: absence of the banner is how "no row needs an answer"
 * is shown, so reusing it here would report a check that never ran as a check that passed. Neutral,
 * not alarming - an unreadable review says nothing about the rows themselves.
 */
function LocationReviewUnavailable() {
  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <AlertTriangle className="h-4 w-4 text-muted" />
          Location review unavailable
        </CardTitle>
        <CardDescription>
          Rows stored at exactly <code>0, 0</code> could not be listed, so this page cannot say
          whether any need an operator's answer. It is not a statement that none do.
        </CardDescription>
      </CardHeader>
    </Card>
  );
}

/**
 * Rows stored at exactly `0, 0`, which only an operator can resolve.
 *
 * It offers no "fix it" button on purpose. `0, 0` is a legal coordinate AND the value every blank
 * location was stored as before location became optional, so nothing here can tell a provider
 * genuinely in the Gulf of Guinea from one that simply has no address - and a guess would either
 * plant a false pin or erase a real one. The banner states the question; the answer comes from a
 * human who knows the provider.
 */
function LocationReviewBanner({ review }: { review: BusinessLocationReviewResp }) {
  return (
    <Card className="border-warning/40">
      <CardHeader>
        <CardTitle className="flex items-center gap-2 text-base">
          <AlertTriangle className="h-4 w-4 text-warning" />
          {review.needsReview} of {review.totalBusinesses}{" "}
          {review.totalBusinesses === 1 ? "row" : "rows"}{" "}
          {review.needsReview === 1 ? "needs" : "need"} a location answer
        </CardTitle>
        <CardDescription>
          These are stored at exactly <code>0, 0</code> - a legal coordinate in the Gulf of Guinea,
          and also the value a blank location was stored as before location became optional. Nothing
          here can tell the two apart, so nothing here changes them. Each row needs an operator to
          say <em>this pin is correct</em>, <em>this pin is wrong, here is the right one</em>, or{" "}
          <em>this provider has no location</em>.
        </CardDescription>
      </CardHeader>
      <CardContent>
        <Table>
          <TableHeader>
            <TableRow>
              <TableHead>Name</TableHead>
              <TableHead>Type</TableHead>
              <TableHead>Domain</TableHead>
              <TableHead>Reachable without a location?</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {review.businesses.map((b) => (
              <TableRow key={b.businessId}>
                <TableCell className="font-medium">{b.name}</TableCell>
                <TableCell>
                  <Badge variant="neutral">{b.type}</Badge>
                </TableCell>
                <TableCell className="font-mono text-xs">{b.domain}</TableCell>
                <TableCell className="text-xs text-muted">
                  {b.hasContact ? "Yes - has contact channels" : "No - no contact channel either"}
                </TableCell>
              </TableRow>
            ))}
          </TableBody>
        </Table>
      </CardContent>
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
    ...blankContactFields(),
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

    // ONE shared rule, also used by the setup Wizard's register step: blank means ABSENT, never the
    // `Number("") === 0` that used to plant a pin at 0,0. The server enforces it independently.
    const location = parseLocationInput(form.lat, form.lng);
    if (location.kind === "invalid") {
      toast({ title: "Check the location", description: location.reason, variant: "danger" });
      return;
    }

    setBusy(true);
    try {
      const r = await central.registerBusiness({
        type: form.type,
        name: form.name,
        ...locationRequestFields(location),
        contact: contactRequestFields(form),
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
              <Sparkles className="h-4 w-4" /> Fill demo data (vet)
            </Button>
            <Button type="button" variant="outline" size="sm" onClick={() => setForm({ ...DEMO_BUSINESS_GROOMER })}>
              <Sparkles className="h-4 w-4" /> Fill demo data (groomer)
            </Button>
            <Button type="button" variant="outline" size="sm" onClick={() => setForm({ ...DEMO_BUSINESS_CONTACT_ONLY })}>
              <Sparkles className="h-4 w-4" /> Fill demo data (no location)
            </Button>
          </div>
        )}
        <form onSubmit={submit} className="grid gap-3 sm:grid-cols-2">
          <Field label="Type" value={form.type} onChange={(v) => upd("type", v)} placeholder="groomer / vet" required />
          <Field label="Name" value={form.name} onChange={(v) => upd("name", v)} required />
          {/* Deliberately NOT required, and deliberately both-or-neither. Leaving them blank
              registers a provider with no location; it used to store 0,0 and draw a pin there. */}
          <Field label="Latitude" value={form.lat} onChange={(v) => upd("lat", v)} placeholder="37.7749 (optional)" />
          <Field label="Longitude" value={form.lng} onChange={(v) => upd("lng", v)} placeholder="-122.4194 (optional)" />
          <p className="text-xs text-muted sm:col-span-2">
            Location is optional - leave both blank for a provider with no address. It is then listed
            and reached through the contact channels below, and never given a false coordinate.
          </p>
          {CONTACT_FIELDS.map(({ key, label, placeholder }) => (
            <Field
              key={key}
              label={label}
              value={form[key]}
              onChange={(v) => upd(key, v)}
              placeholder={placeholder}
            />
          ))}
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

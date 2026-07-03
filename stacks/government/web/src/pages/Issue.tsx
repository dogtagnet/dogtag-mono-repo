import { Badge, Button, Card, Input } from "@dogtag/ui";
import { useState } from "react";
import { Link } from "react-router-dom";
import { apiPost, publicReceiptUrl } from "../lib/api";

/** One applicant field (maps 1:1 to the `credentialSubject` leaf the backend builds in build_gov_vc,
 *  e.g. `importerLastName` → credentialSubject.importer.lastName). */
interface FieldSpec {
  key: string;
  label: string;
  placeholder?: string;
}

interface FieldSection {
  title: string;
  hint?: string;
  fields: FieldSpec[];
}

/** Per-record-type Issue form, grouped into the CDC receipt's own sections (A person / B animal /
 *  C travel / validity) for TRAVEL_CLEARANCE, and the Annex-IV clinical block for EU_HEALTH_CERT.
 *  Blank fields fall back to the backend's per-type defaults, so a minimal demo issue still works. */
const RECORD_TYPE_SECTIONS: Record<string, FieldSection[]> = {
  TRAVEL_CLEARANCE: [
    {
      title: "Section A — Person importing the animal",
      hint: "PII (the private / obfuscatable block — committed in R as salted leaves, never on-chain in the clear).",
      fields: [
        { key: "importerFirstName", label: "First name", placeholder: "Dominic" },
        { key: "importerMiddleName", label: "Middle name/initial", placeholder: "" },
        { key: "importerLastName", label: "Last name", placeholder: "Zagara" },
        { key: "importerRole", label: "The person listed above is the", placeholder: "owner" },
        { key: "importerIdType", label: "Identification type", placeholder: "drivers_license" },
        { key: "importerIdJurisdiction", label: "ID jurisdiction (state/country)", placeholder: "US-NY" },
        { key: "importerIdNumber", label: "Identification number", placeholder: "887524355" },
        { key: "importerDateOfBirth", label: "Date of birth", placeholder: "1997-02-13" },
        { key: "importerEmail", label: "Email", placeholder: "dom.zagara@example.com" },
        { key: "importerPhone", label: "Phone number", placeholder: "216-533-5925" },
      ],
    },
    {
      title: "Section B — Animal information",
      fields: [
        { key: "animalName", label: "Animal name", placeholder: "Blaze" },
        { key: "animalAgeYears", label: "Age — year(s)", placeholder: "3" },
        { key: "animalAgeMonths", label: "Age — month(s)", placeholder: "1" },
        { key: "animalSex", label: "Sex", placeholder: "male" },
        { key: "animalNeutered", label: "Neutered (true/false)", placeholder: "true" },
        { key: "animalBreed", label: "Dog breed", placeholder: "Poodle - Standard" },
        { key: "animalColorMarkings", label: "Color/markings", placeholder: "Grey" },
        { key: "importationPurpose", label: "Importation purpose", placeholder: "service_animal" },
      ],
    },
    {
      title: "Section C — Travel information",
      fields: [
        { key: "travelType", label: "Travel type", placeholder: "air" },
        { key: "countryOfDeparture", label: "Country/area of departure", placeholder: "CA" },
        { key: "dateOfArrival", label: "Date of arrival", placeholder: "2026-07-08" },
        { key: "portOfEntry", label: "Port of entry", placeholder: "JFK" },
        { key: "carrierOrFlight", label: "Carrier / flight", placeholder: "AC 8552" },
      ],
    },
    {
      title: "Validity",
      hint: "Issuance DATE is derived from the anchoring block on-chain — not entered here.",
      fields: [
        { key: "validFrom", label: "Valid from", placeholder: "2026-07-01" },
        { key: "validUntil", label: "Valid until", placeholder: "2027-01-01" },
        { key: "multipleEntries", label: "Multiple entries (true/false)", placeholder: "true" },
        { key: "countryOfDepartureBinding", label: "Country-of-departure binding", placeholder: "CA" },
      ],
    },
  ],
  EU_HEALTH_CERT: [
    {
      title: "Health certificate (Annex IV)",
      fields: [
        { key: "species", label: "Species", placeholder: "dog" },
        { key: "microchipNumber", label: "Microchip number", placeholder: "985112345678903" },
        { key: "rabiesVaccinationDate", label: "Rabies vaccination date", placeholder: "2026-01-15" },
        { key: "rabiesValidUntil", label: "Rabies valid until", placeholder: "2029-01-14" },
        { key: "examiningVeterinarian", label: "Examining veterinarian", placeholder: "Dr. A. Meyer, DVM" },
        { key: "clinicalHealthStatus", label: "Clinical health status", placeholder: "fit_for_travel" },
        { key: "examinationDate", label: "Examination date", placeholder: "2026-02-01" },
      ],
    },
  ],
};

function CopyButton({ text, label }: { text: string; label: string }) {
  const [copied, setCopied] = useState(false);
  async function copy() {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // Fallback for browsers/contexts without the async clipboard API.
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      try {
        document.execCommand("copy");
      } finally {
        document.body.removeChild(ta);
      }
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 1600);
  }
  return (
    <Button type="button" variant="outline" size="sm" data-testid="copy-wrapped" onClick={copy}>
      {copied ? "✓ Copied" : label}
    </Button>
  );
}

interface IssueResult {
  wrappedDoc?: unknown;
  root?: string;
  receiptId?: string;
  anchored?: boolean;
  txHash?: string | null;
  explorerUrl?: string | null;
  [k: string]: unknown;
}

export function Issue() {
  const [recordType, setRecordType] = useState("TRAVEL_CLEARANCE");
  const [dogTagId, setDogTagId] = useState("7");
  // One value map covering every record type's fields; only the active type's fields are rendered.
  const [values, setValues] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<IssueResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const sections = RECORD_TYPE_SECTIONS[recordType] ?? [];

  function setField(key: string, v: string) {
    setValues((prev) => ({ ...prev, [key]: v }));
  }

  async function submit() {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      // Send only the fields defined for the selected record type (blank => backend default).
      const fields: Record<string, string> = {};
      for (const s of sections) {
        for (const f of s.fields) {
          const v = values[f.key];
          if (v != null && v.trim() !== "") fields[f.key] = v.trim();
        }
      }
      const { status, json } = await apiPost(
        "/v1/travel-clearance/issue",
        { record_type: recordType, dog_tag_id: dogTagId, fields },
        { auth: true }, // issue is gated by the operator bearer (arch DP-6)
      );
      if (status !== 200) setError(json.error || `HTTP ${status}`);
      else setResult(json);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const wrappedDoc = result?.wrappedDoc;
  const wrappedJson = wrappedDoc != null ? JSON.stringify(wrappedDoc, null, 2) : "";
  const receiptId = result?.receiptId;

  return (
    <Card className="p-6">
      <h2 className="text-base font-semibold text-onSurface">Issue authority-endorsed credential</h2>
      <p className="mt-1 text-sm text-muted">
        Builds a salted Poseidon-Merkle credential (single root R) and anchors it on the ROAX
        DogTagIssuer clone. Trust tier: accredited_authority.
      </p>

      <label className="mt-4 block text-xs font-medium text-muted">Record type</label>
      <select
        data-testid="record-type"
        value={recordType}
        onChange={(e) => setRecordType(e.target.value)}
        className="mt-1 flex h-10 w-full rounded-md border border-input bg-surface px-3 text-sm text-onSurface focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      >
        <option value="TRAVEL_CLEARANCE">TRAVEL_CLEARANCE</option>
        <option value="EU_HEALTH_CERT">EU_HEALTH_CERT</option>
      </select>

      <label className="mt-4 block text-xs font-medium text-muted">
        Dog tag id (SBT tokenId handle)
      </label>
      <Input className="mt-1" value={dogTagId} onChange={(e) => setDogTagId(e.target.value)} />

      {sections.map((section) => (
        <div key={section.title} className="mt-6">
          <div className="rounded-t-md bg-secondary px-3 py-2 text-sm font-semibold text-onSecondary">
            {section.title}
          </div>
          {section.hint && (
            <p className="mt-1 text-xs text-muted">{section.hint}</p>
          )}
          <div className="mt-3 grid grid-cols-1 gap-3 sm:grid-cols-2">
            {section.fields.map((f) => (
              <div key={f.key}>
                <label className="block text-xs font-medium text-muted">{f.label}</label>
                <Input
                  className="mt-1"
                  data-testid={`field-${f.key}`}
                  value={values[f.key] ?? ""}
                  placeholder={f.placeholder}
                  onChange={(e) => setField(f.key, e.target.value)}
                />
              </div>
            ))}
          </div>
        </div>
      ))}

      <div className="mt-6">
        <Button data-testid="issue-submit" loading={busy} disabled={busy} onClick={submit}>
          {busy ? "Issuing…" : "Issue + anchor"}
        </Button>
      </div>

      {error && (
        <p className="mt-4">
          <Badge variant="danger">{error}</Badge>
        </p>
      )}

      {result != null && (
        <div className="mt-5 space-y-4">
          <div className="flex flex-wrap items-center gap-2">
            <Badge variant={result.anchored ? "success" : "neutral"}>
              {result.anchored ? "✓ anchored on-chain" : "built (not anchored)"}
            </Badge>
            {result.root && (
              <span className="receipt-mono text-xs text-muted">
                root {String(result.root).slice(0, 14)}…
              </span>
            )}
          </div>

          {receiptId && result.root && (
            <Card className="border-primary/40 bg-primary/5 p-4">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div>
                  <div className="text-sm font-semibold text-onSurface">
                    Receipt {receiptId}
                  </div>
                  <div className="mt-0.5 text-xs text-muted">
                    Official CDC-modeled travel-clearance receipt (printable / phone-showable).
                  </div>
                </div>
                <Button asChild size="sm">
                  <Link data-testid="view-receipt" to={`/receipt/${result.root}`}>
                    View / print receipt →
                  </Link>
                </Button>
              </div>
              <div className="mt-3 text-xs text-muted">
                Public status page (PII-free, what the QR encodes):{" "}
                <a
                  className="text-primary underline-offset-2 hover:underline"
                  href={publicReceiptUrl(receiptId)}
                  target="_blank"
                  rel="noreferrer"
                >
                  {publicReceiptUrl(receiptId)}
                </a>
              </div>
            </Card>
          )}

          {wrappedJson && (
            <Card className="border-accent/40 p-4">
              <div className="flex items-center justify-between gap-3">
                <strong className="text-sm text-onSurface">Wrapped credential document</strong>
                <CopyButton text={wrappedJson} label="Copy wrapped document" />
              </div>
              <p className="mt-1 text-xs text-muted">
                Copy this and paste it into the <strong>Verify</strong> tab to check the three
                authenticity pillars.
              </p>
              <pre
                data-testid="wrapped-doc"
                className="receipt-mono mt-3 max-h-80 overflow-auto rounded-md border border-border bg-surface-muted p-3 text-xs text-onSurface"
              >
                {wrappedJson}
              </pre>
            </Card>
          )}

          <details>
            <summary className="cursor-pointer text-xs text-muted">Full issue response</summary>
            <pre className="receipt-mono mt-2 max-h-80 overflow-auto rounded-md border border-border bg-surface-muted p-3 text-xs text-onSurface">
              {JSON.stringify(result, null, 2)}
            </pre>
          </details>
        </div>
      )}
    </Card>
  );
}

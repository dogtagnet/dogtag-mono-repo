import { useEffect, useState } from "react";
import { NavLink, Navigate, Route, Routes } from "react-router-dom";

const API_BASE = import.meta.env.VITE_GOV_API_BASE || "/api";

async function apiGet(path: string) {
  const r = await fetch(`${API_BASE}${path}`);
  return r.json();
}
async function apiPost(path: string, body: unknown) {
  const r = await fetch(`${API_BASE}${path}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
  });
  return { status: r.status, json: await r.json() };
}

interface Health {
  status?: string;
  chainId?: number;
  demo?: boolean;
  canSign?: boolean;
  signer?: string | null;
}

function Header({ health }: { health: Health | null }) {
  return (
    <header className="top">
      <h1>DogTag Government Authority</h1>
      <span className="badge">{health?.demo ? "DEMO" : "LIVE"}</span>
      <nav>
        <NavLink to="/issue" className={({ isActive }) => (isActive ? "active" : "")}>
          Issue
        </NavLink>
        <NavLink to="/verify" className={({ isActive }) => (isActive ? "active" : "")}>
          Verify
        </NavLink>
      </nav>
    </header>
  );
}

interface FieldSpec {
  key: string;
  label: string;
  default: string;
}

// Each government credential type has its OWN field set. A travel clearance and an EU Annex-IV
// health certificate attest fundamentally different things, so their Issue forms differ.
const FIELD_SETS: Record<string, FieldSpec[]> = {
  TRAVEL_CLEARANCE: [
    { key: "originCountry", label: "Origin country", default: "US" },
    { key: "destinationCountry", label: "Destination country", default: "DE" },
    { key: "purposeOfMovement", label: "Purpose of movement", default: "non-commercial" },
    { key: "clearanceReference", label: "Clearance reference", default: "GOV-TC-2026-0001" },
    { key: "validFrom", label: "Valid from", default: "2026-01-01" },
    { key: "validUntil", label: "Valid until", default: "2026-05-01" },
    { key: "endorsingAuthority", label: "Endorsing authority", default: "Example Competent Authority" },
  ],
  EU_HEALTH_CERT: [
    { key: "species", label: "Species", default: "Canis lupus familiaris" },
    { key: "breed", label: "Breed", default: "Mixed Breed" },
    { key: "sex", label: "Sex", default: "male" },
    { key: "microchipNumber", label: "Microchip number", default: "985141006580319" },
    { key: "microchipImplantDate", label: "Microchip implant date", default: "2025-06-01" },
    { key: "rabiesVaccinationDate", label: "Rabies vaccination date", default: "2026-01-11" },
    { key: "rabiesValidUntil", label: "Rabies valid until", default: "2027-01-10" },
    { key: "clinicalExaminationDate", label: "Clinical examination date", default: "2026-01-20" },
    { key: "destinationMemberState", label: "Destination member state", default: "DE" },
    { key: "certificateNumber", label: "Certificate number", default: "EU-HC-2026-0001" },
    { key: "officialVeterinarian", label: "Official veterinarian", default: "Dr. A. Muster" },
  ],
};

function defaultsFor(recordType: string): Record<string, string> {
  const out: Record<string, string> = {};
  for (const f of FIELD_SETS[recordType] ?? []) out[f.key] = f.default;
  return out;
}

function IssuePage() {
  const [recordType, setRecordType] = useState("TRAVEL_CLEARANCE");
  const [dogTagId, setDogTagId] = useState("7");
  const [values, setValues] = useState<Record<string, string>>(() => defaultsFor("TRAVEL_CLEARANCE"));
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  function onTypeChange(rt: string) {
    setRecordType(rt);
    setValues(defaultsFor(rt)); // reset to the new type's own field set
    setResult(null);
    setError(null);
  }

  async function submit() {
    setBusy(true);
    setError(null);
    setResult(null);
    setCopied(false);
    try {
      const { status, json } = await apiPost("/v1/travel-clearance/issue", {
        record_type: recordType,
        dog_tag_id: dogTagId,
        fields: values,
      });
      if (status !== 200) setError(json.error || `HTTP ${status}`);
      else setResult(json);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const wrappedDoc = result?.wrappedDoc;
  const wrappedJson = wrappedDoc ? JSON.stringify(wrappedDoc) : "";

  async function copyDoc() {
    try {
      await navigator.clipboard.writeText(wrappedJson);
      setCopied(true);
      setTimeout(() => setCopied(false), 2500);
    } catch {
      setCopied(false);
    }
  }

  const fields = FIELD_SETS[recordType] ?? [];

  return (
    <div className="card">
      <h2>Issue authority-endorsed credential</h2>
      <p className="sub">
        Builds a salted Poseidon-Merkle credential (single root R) and anchors it on the ROAX
        DogTagIssuer clone. Trust tier: accredited_authority.
      </p>
      <label>Record type</label>
      <select
        data-testid="record-type"
        value={recordType}
        onChange={(e) => onTypeChange(e.target.value)}
      >
        <option value="TRAVEL_CLEARANCE">TRAVEL_CLEARANCE — pet travel clearance</option>
        <option value="EU_HEALTH_CERT">EU_HEALTH_CERT — EU Annex IV health certificate</option>
      </select>
      <label>Dog tag id (SBT tokenId handle)</label>
      <input data-testid="dog-tag-id" value={dogTagId} onChange={(e) => setDogTagId(e.target.value)} />
      <div className="row">
        {fields.map((f) => (
          <div key={f.key}>
            <label>{f.label}</label>
            <input
              data-testid={`field-${f.key}`}
              value={values[f.key] ?? ""}
              onChange={(e) => setValues((v) => ({ ...v, [f.key]: e.target.value }))}
            />
          </div>
        ))}
      </div>
      <button data-testid="issue-btn" disabled={busy} onClick={submit}>
        {busy ? "Issuing…" : "Issue + anchor"}
      </button>
      {error && <p className="pill bad" style={{ marginTop: 14 }}>{error}</p>}
      {result && (
        <div style={{ marginTop: 18 }}>
          <div className="verdict ok">✓ Issued</div>
          <div className="muted" style={{ marginTop: 6, wordBreak: "break-all" }}>
            root {String(result.root)} · {result.anchored ? "anchored on-chain" : "built (dry-run)"}
            {result.txHash ? ` · tx ${String(result.txHash)}` : ""}
          </div>
          <div style={{ display: "flex", alignItems: "center", gap: 12, margin: "14px 0 6px" }}>
            <strong style={{ fontSize: 14 }}>Wrapped credential document</strong>
            <button
              data-testid="copy-btn"
              style={{ marginTop: 0, padding: "6px 12px", fontSize: 13 }}
              onClick={copyDoc}
            >
              {copied ? "✓ Copied" : "Copy wrapped document"}
            </button>
            <span className="muted">paste it into the Verify tab</span>
          </div>
          <textarea
            data-testid="wrapped-doc"
            readOnly
            value={JSON.stringify(wrappedDoc, null, 2)}
            style={{ minHeight: 200 }}
          />
        </div>
      )}
    </div>
  );
}

function VerifyPage() {
  const [doc, setDoc] = useState("");
  const [signer, setSigner] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Pre-fill the issuer signer with this authority's own signer so the issuer-identity pillar
  // resolves without the operator having to know the address (they can still override it).
  useEffect(() => {
    apiGet("/health")
      .then((h: Health) => {
        if (h?.signer) setSigner(h.signer);
      })
      .catch(() => {});
  }, []);

  async function submit() {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const wrapped = JSON.parse(doc);
      const body: Record<string, unknown> = { wrapped_doc: wrapped };
      if (signer.trim()) body.signer_addr = signer.trim();
      const { status, json } = await apiPost("/v1/verify", body);
      if (status !== 200) setError(json.error || `HTTP ${status}`);
      else setResult(json);
    } catch (e) {
      setError(`Invalid JSON or request: ${e}`);
    } finally {
      setBusy(false);
    }
  }

  const frag = result?.fragments as Record<string, unknown> | undefined;
  const verdict = result?.verdict as boolean | undefined;

  return (
    <div className="card">
      <h2>Verify a credential</h2>
      <p className="sub">
        Recomputes integrity (offline) and reads on-chain status (DogTagIssuer.isValid) + issuer
        identity (IssuerRegistry.isWhitelistedFor) from ROAX. All reads are gasless.
      </p>
      <label>Wrapped credential document (JSON)</label>
      <textarea
        data-testid="verify-doc"
        placeholder='Paste the wrappedDoc returned by an issuer…'
        value={doc}
        onChange={(e) => setDoc(e.target.value)}
      />
      <label>Issuer signer address (checks the whitelist pillar)</label>
      <input data-testid="verify-signer" value={signer} onChange={(e) => setSigner(e.target.value)} placeholder="0x…" />
      <button data-testid="verify-btn" disabled={busy || !doc.trim()} onClick={submit}>
        {busy ? "Verifying…" : "Verify"}
      </button>
      {error && <p className="pill bad" style={{ marginTop: 14 }}>{error}</p>}
      {result && (
        <div style={{ marginTop: 16 }}>
          <div data-testid="verdict" className={`verdict ${verdict ? "ok" : "bad"}`}>
            {verdict ? "✓ VALID" : "✗ INVALID"}
          </div>
          <div style={{ marginTop: 10 }}>
            <Frag label="integrity" v={frag?.integrity} />
            <Frag label="on-chain" v={frag?.onchain} />
            <Frag label="issuer whitelist" v={frag?.issuerWhitelisted} />
          </div>
          <pre style={{ marginTop: 14 }}>{JSON.stringify(result, null, 2)}</pre>
        </div>
      )}
    </div>
  );
}

function Frag({ label, v }: { label: string; v: unknown }) {
  const tid = `frag-${label.replace(/\s+/g, "-")}`;
  if (v === null || v === undefined)
    return <span data-testid={tid} data-value="na" className="pill">{label}: n/a</span>;
  return (
    <span data-testid={tid} data-value={v ? "yes" : "no"} className={`pill ${v ? "ok" : "bad"}`}>
      {label}: {v ? "yes" : "no"}
    </span>
  );
}

export function App() {
  const [health, setHealth] = useState<Health | null>(null);
  useEffect(() => {
    apiGet("/health").then(setHealth).catch(() => setHealth(null));
  }, []);
  return (
    <div className="wrap">
      <Header health={health} />
      <Routes>
        <Route path="/" element={<Navigate to="/issue" replace />} />
        <Route path="/issue" element={<IssuePage />} />
        <Route path="/verify" element={<VerifyPage />} />
      </Routes>
      <p className="muted">
        chainId {health?.chainId ?? "?"} · signer {health?.signer ?? "none"} ·{" "}
        {health?.canSign ? "can anchor on-chain" : "read-only (no signer)"}
      </p>
    </div>
  );
}

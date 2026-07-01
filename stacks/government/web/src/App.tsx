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

function IssuePage() {
  const [recordType, setRecordType] = useState("TRAVEL_CLEARANCE");
  const [dogTagId, setDogTagId] = useState("7");
  const [origin, setOrigin] = useState("US");
  const [destination, setDestination] = useState("DE");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<unknown>(null);
  const [error, setError] = useState<string | null>(null);

  async function submit() {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const { status, json } = await apiPost("/v1/travel-clearance/issue", {
        record_type: recordType,
        dog_tag_id: dogTagId,
        fields: { originCountry: origin, destinationCountry: destination },
      });
      if (status !== 200) setError(json.error || `HTTP ${status}`);
      setResult(json);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="card">
      <h2>Issue authority-endorsed credential</h2>
      <p className="sub">
        Builds a salted Poseidon-Merkle credential (single root R) and anchors it on the ROAX
        DogTagIssuer clone. Trust tier: accredited_authority.
      </p>
      <label>Record type</label>
      <select value={recordType} onChange={(e) => setRecordType(e.target.value)}>
        <option value="TRAVEL_CLEARANCE">TRAVEL_CLEARANCE</option>
        <option value="EU_HEALTH_CERT">EU_HEALTH_CERT</option>
      </select>
      <label>Dog tag id (SBT tokenId handle)</label>
      <input value={dogTagId} onChange={(e) => setDogTagId(e.target.value)} />
      <div className="row">
        <div>
          <label>Origin country</label>
          <input value={origin} onChange={(e) => setOrigin(e.target.value)} />
        </div>
        <div>
          <label>Destination country</label>
          <input value={destination} onChange={(e) => setDestination(e.target.value)} />
        </div>
      </div>
      <button disabled={busy} onClick={submit}>
        {busy ? "Issuing…" : "Issue + anchor"}
      </button>
      {error && <p className="pill bad" style={{ marginTop: 14 }}>{error}</p>}
      {result != null && <pre style={{ marginTop: 16 }}>{JSON.stringify(result, null, 2)}</pre>}
    </div>
  );
}

function VerifyPage() {
  const [doc, setDoc] = useState("");
  const [signer, setSigner] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState<string | null>(null);

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
        placeholder='Paste the wrappedDoc returned by an issuer…'
        value={doc}
        onChange={(e) => setDoc(e.target.value)}
      />
      <label>Issuer signer address (optional — checks the whitelist pillar)</label>
      <input value={signer} onChange={(e) => setSigner(e.target.value)} placeholder="0x…" />
      <button disabled={busy || !doc.trim()} onClick={submit}>
        {busy ? "Verifying…" : "Verify"}
      </button>
      {error && <p className="pill bad" style={{ marginTop: 14 }}>{error}</p>}
      {result && (
        <div style={{ marginTop: 16 }}>
          <div className={`verdict ${verdict ? "ok" : "bad"}`}>
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
  if (v === null || v === undefined) return <span className="pill">{label}: n/a</span>;
  return <span className={`pill ${v ? "ok" : "bad"}`}>{label}: {v ? "yes" : "no"}</span>;
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

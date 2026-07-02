import { useEffect, useState } from "react";
import { NavLink, Navigate, Route, Routes } from "react-router-dom";

const API_BASE = import.meta.env.VITE_GOV_API_BASE || "/api";
// Bearer token for the record MUTATION endpoints (PATCH + revoke). The fallback matches the
// backend's demo-mode default so the demo portal works out of the box; set VITE_GOV_API_TOKEN
// (and GOV_API_TOKEN on the API) for a real deployment.
const API_TOKEN = import.meta.env.VITE_GOV_API_TOKEN || "dogtag-gov-demo-token";

async function apiGet(path: string) {
  const r = await fetch(`${API_BASE}${path}`);
  return r.json();
}
async function apiPost(path: string, body: unknown, opts?: { auth?: boolean }) {
  const headers: Record<string, string> = { "content-type": "application/json" };
  if (opts?.auth) headers.authorization = `Bearer ${API_TOKEN}`;
  const r = await fetch(`${API_BASE}${path}`, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
  });
  return { status: r.status, json: await r.json() };
}
async function apiPatch(path: string, body: unknown) {
  const r = await fetch(`${API_BASE}${path}`, {
    method: "PATCH",
    headers: { "content-type": "application/json", authorization: `Bearer ${API_TOKEN}` },
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
        <NavLink to="/records" className={({ isActive }) => (isActive ? "active" : "")}>
          Records
        </NavLink>
      </nav>
    </header>
  );
}

/** Per-record-type Issue-form field schemas. Each government credential type has its OWN correct
 *  set of applicant fields — a travel clearance describes a cross-border movement, an EU health
 *  certificate (Annex IV) describes the animal's clinical/vaccination status. The `key` maps 1:1
 *  onto the `credentialSubject` leaf the backend builds in `build_gov_vc`. */
interface FieldSpec {
  key: string;
  label: string;
  placeholder?: string;
}

const RECORD_TYPE_FIELDS: Record<string, FieldSpec[]> = {
  TRAVEL_CLEARANCE: [
    { key: "originCountry", label: "Origin country", placeholder: "US" },
    { key: "destinationCountry", label: "Destination country", placeholder: "DE" },
    { key: "purposeOfMovement", label: "Purpose of movement", placeholder: "non-commercial" },
    { key: "clearanceReference", label: "Clearance reference", placeholder: "GOV-7" },
    { key: "validFrom", label: "Valid from", placeholder: "2026-01-01" },
    { key: "validUntil", label: "Valid until", placeholder: "2026-05-01" },
  ],
  EU_HEALTH_CERT: [
    { key: "species", label: "Species", placeholder: "dog" },
    { key: "microchipNumber", label: "Microchip number", placeholder: "985112345678903" },
    { key: "rabiesVaccinationDate", label: "Rabies vaccination date", placeholder: "2026-01-15" },
    { key: "rabiesValidUntil", label: "Rabies valid until", placeholder: "2029-01-14" },
    { key: "examiningVeterinarian", label: "Examining veterinarian", placeholder: "Dr. A. Meyer, DVM" },
    { key: "clinicalHealthStatus", label: "Clinical health status", placeholder: "fit_for_travel" },
    { key: "examinationDate", label: "Examination date", placeholder: "2026-02-01" },
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
    <button type="button" className="copy-btn" data-testid="copy-wrapped" onClick={copy}>
      {copied ? "✓ Copied" : label}
    </button>
  );
}

interface IssueResult {
  wrappedDoc?: unknown;
  root?: string;
  anchored?: boolean;
  txHash?: string | null;
  [k: string]: unknown;
}

function IssuePage() {
  const [recordType, setRecordType] = useState("TRAVEL_CLEARANCE");
  const [dogTagId, setDogTagId] = useState("7");
  // One value map covering every record type's fields; only the active type's fields are rendered.
  const [values, setValues] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<IssueResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const specs = RECORD_TYPE_FIELDS[recordType] ?? [];

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
      for (const s of specs) {
        const v = values[s.key];
        if (v != null && v.trim() !== "") fields[s.key] = v.trim();
      }
      const { status, json } = await apiPost("/v1/travel-clearance/issue", {
        record_type: recordType,
        dog_tag_id: dogTagId,
        fields,
      });
      if (status !== 200) setError(json.error || `HTTP ${status}`);
      setResult(json);
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }

  const wrappedDoc = result?.wrappedDoc;
  const wrappedJson = wrappedDoc != null ? JSON.stringify(wrappedDoc, null, 2) : "";

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
        onChange={(e) => setRecordType(e.target.value)}
      >
        <option value="TRAVEL_CLEARANCE">TRAVEL_CLEARANCE</option>
        <option value="EU_HEALTH_CERT">EU_HEALTH_CERT</option>
      </select>
      <label>Dog tag id (SBT tokenId handle)</label>
      <input value={dogTagId} onChange={(e) => setDogTagId(e.target.value)} />
      <div className="fields">
        {specs.map((s) => (
          <div key={s.key}>
            <label>{s.label}</label>
            <input
              data-testid={`field-${s.key}`}
              value={values[s.key] ?? ""}
              placeholder={s.placeholder}
              onChange={(e) => setField(s.key, e.target.value)}
            />
          </div>
        ))}
      </div>
      <button data-testid="issue-submit" disabled={busy} onClick={submit}>
        {busy ? "Issuing…" : "Issue + anchor"}
      </button>
      {error && <p className="pill bad" style={{ marginTop: 14 }}>{error}</p>}
      {result != null && (
        <div style={{ marginTop: 18 }}>
          <div className="pill ok">
            {result.anchored ? "✓ anchored on-chain" : "built (not anchored)"}
            {result.root ? ` · root ${String(result.root).slice(0, 12)}…` : ""}
          </div>
          {wrappedJson && (
            <div className="wrapped-doc">
              <div className="wrapped-doc-head">
                <strong>Wrapped credential document</strong>
                <CopyButton text={wrappedJson} label="Copy wrapped document" />
              </div>
              <p className="sub" style={{ margin: "6px 0 10px" }}>
                Copy this and paste it into the <strong>Verify</strong> tab to check the three
                authenticity pillars.
              </p>
              <pre data-testid="wrapped-doc" className="wrapped-doc-body">
                {wrappedJson}
              </pre>
            </div>
          )}
          <details style={{ marginTop: 12 }}>
            <summary className="muted">Full issue response</summary>
            <pre style={{ marginTop: 8 }}>{JSON.stringify(result, null, 2)}</pre>
          </details>
        </div>
      )}
    </div>
  );
}

function VerifyPage({ health }: { health: Health | null }) {
  const [doc, setDoc] = useState("");
  const [signer, setSigner] = useState("");
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<Record<string, unknown> | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Prefill the issuer signer with this authority's own signer so the whitelist pillar (the third
  // authenticity pillar) is exercised by default. Users can override or clear it.
  useEffect(() => {
    if (health?.signer && !signer) setSigner(health.signer);
  }, [health?.signer]);

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
      <label>Issuer signer address (optional — checks the whitelist pillar)</label>
      <input
        data-testid="verify-signer"
        value={signer}
        onChange={(e) => setSigner(e.target.value)}
        placeholder="0x…"
      />
      <button data-testid="verify-submit" disabled={busy || !doc.trim()} onClick={submit}>
        {busy ? "Verifying…" : "Verify"}
      </button>
      {error && <p className="pill bad" style={{ marginTop: 14 }}>{error}</p>}
      {result && (
        <div style={{ marginTop: 16 }}>
          <div data-testid="verdict" className={`verdict ${verdict ? "ok" : "bad"}`}>
            {verdict ? "✓ VALID" : "✗ INVALID"}
          </div>
          <div style={{ marginTop: 10 }}>
            <Frag label="integrity" testid="pillar-integrity" v={frag?.integrity} />
            <Frag label="on-chain" testid="pillar-onchain" v={frag?.onchain} />
            <Frag label="issuer whitelist" testid="pillar-whitelist" v={frag?.issuerWhitelisted} />
          </div>
          <pre style={{ marginTop: 14 }}>{JSON.stringify(result, null, 2)}</pre>
        </div>
      )}
    </div>
  );
}

function Frag({ label, v, testid }: { label: string; v: unknown; testid?: string }) {
  if (v === null || v === undefined)
    return <span data-testid={testid} className="pill">{label}: n/a</span>;
  return (
    <span data-testid={testid} className={`pill ${v ? "ok" : "bad"}`}>
      {label}: {v ? "yes" : "no"}
    </span>
  );
}

/** A persisted government credential as returned by GET /v1/records (serde camelCase). */
interface GovRecord {
  root: string;
  recordType: string;
  dogTagId: string;
  issuerAddr: string;
  status: string;
  txHash?: string | null;
  blockNumber?: number | null;
  explorerUrl?: string | null;
  anchored?: boolean;
  label?: string | null;
  notes?: string | null;
  revokedTxHash?: string | null;
  revokeExplorerUrl?: string | null;
  invalidationReason?: string | null;
  createdAt?: number;
  updatedAt?: number;
}

/** The authority's own credential ledger: every issued credential with its immutable on-chain proof
 *  (tx / block / contract / explorer link), editable off-chain metadata, and soft-invalidation. */
function RecordsPage() {
  const [records, setRecords] = useState<GovRecord[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editRoot, setEditRoot] = useState<string | null>(null);
  const [editLabel, setEditLabel] = useState("");
  const [editNotes, setEditNotes] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  async function refresh() {
    setLoading(true);
    try {
      const json = await apiGet("/v1/records");
      setRecords((json.records as GovRecord[]) ?? []);
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }
  useEffect(() => {
    void refresh();
  }, []);

  function openEdit(rec: GovRecord) {
    setEditRoot(rec.root);
    setEditLabel(rec.label ?? "");
    setEditNotes(rec.notes ?? "");
  }
  async function saveEdit() {
    if (!editRoot) return;
    setBusy(editRoot);
    try {
      const { status, json } = await apiPatch(`/v1/records/${editRoot}`, {
        label: editLabel || null,
        notes: editNotes || null,
      });
      if (status !== 200) setError(json.error || `HTTP ${status}`);
      else {
        setEditRoot(null);
        await refresh();
      }
    } finally {
      setBusy(null);
    }
  }
  async function revoke(rec: GovRecord) {
    if (!window.confirm("Revoke on-chain? It stays on record (as revoked) and remains verifiable.")) return;
    setBusy(rec.root);
    try {
      const { status, json } = await apiPost(
        `/v1/records/${rec.root}/revoke`,
        { reason: "credential withdrawn" },
        { auth: true },
      );
      if (status !== 200) setError(json.error || `HTTP ${status}`);
      await refresh();
    } finally {
      setBusy(null);
    }
  }
  async function markExpired(rec: GovRecord) {
    if (!window.confirm("Mark expired? It stays on record and keeps its on-chain proof.")) return;
    setBusy(rec.root);
    try {
      const { status, json } = await apiPatch(`/v1/records/${rec.root}`, {
        status: "expired",
        reason: "validity window lapsed",
      });
      if (status !== 200) setError(json.error || `HTTP ${status}`);
      await refresh();
    } finally {
      setBusy(null);
    }
  }

  return (
    <div className="card">
      <h2>Issued credentials</h2>
      <p className="sub">
        Every credential this authority issued, from its own database - with the on-chain proof (tx,
        block, contract) and a block-explorer link to verify it. Revoked/expired credentials stay on
        record. Only off-chain metadata is editable; on-chain state is immutable.
      </p>
      <button data-testid="records-refresh" onClick={() => void refresh()} disabled={loading}>
        {loading ? "Loading…" : "Refresh"}
      </button>
      {error && <p className="pill bad" style={{ marginTop: 14 }}>{error}</p>}
      {records.length === 0 ? (
        <p className="muted" style={{ marginTop: 14 }}>
          {loading ? "Loading…" : "No credentials issued yet."}
        </p>
      ) : (
        <table className="records" style={{ marginTop: 14, width: "100%" }}>
          <thead>
            <tr>
              <th>Type</th>
              <th>Dog tag</th>
              <th>Status</th>
              <th>On-chain proof</th>
              <th>Label</th>
              <th></th>
            </tr>
          </thead>
          <tbody>
            {records.map((rec) => (
              <tr key={rec.root} data-testid="record-row" data-status={rec.status}>
                <td>{rec.recordType}</td>
                <td className="mono">{rec.dogTagId}</td>
                <td>
                  <span className={`pill ${rec.status === "issued" ? "ok" : "bad"}`} data-testid="record-status">
                    {rec.status}
                  </span>
                </td>
                <td>
                  {rec.explorerUrl || rec.txHash ? (
                    <div>
                      <a
                        data-testid="explorer-link"
                        href={rec.explorerUrl || `https://explorer.roax.net/tx/${rec.txHash}`}
                        target="_blank"
                        rel="noreferrer"
                        className="mono"
                      >
                        {rec.txHash?.slice(0, 12)}…
                      </a>
                      <div className="muted" style={{ fontSize: 11 }}>
                        block {rec.blockNumber ?? "?"} · {rec.issuerAddr.slice(0, 10)}…
                      </div>
                      {rec.revokeExplorerUrl && (
                        <a
                          data-testid="revoke-explorer-link"
                          href={rec.revokeExplorerUrl}
                          target="_blank"
                          rel="noreferrer"
                          className="mono"
                          style={{ fontSize: 11 }}
                        >
                          revoke tx →
                        </a>
                      )}
                    </div>
                  ) : (
                    <span className="muted">—</span>
                  )}
                </td>
                <td className="muted">{rec.label || "—"}</td>
                <td style={{ textAlign: "right", whiteSpace: "nowrap" }}>
                  <button data-testid="edit-open" onClick={() => openEdit(rec)}>
                    Edit
                  </button>{" "}
                  {rec.status === "issued" && (
                    <>
                      <button data-testid="expire" disabled={busy === rec.root} onClick={() => markExpired(rec)}>
                        Expire
                      </button>{" "}
                    </>
                  )}
                  {(rec.status === "issued" || rec.status === "expired") && (
                    <button data-testid="revoke" disabled={busy === rec.root} onClick={() => revoke(rec)}>
                      Revoke
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}

      {editRoot && (
        <div className="wrapped-doc" style={{ marginTop: 18 }} data-testid="edit-panel">
          <div className="wrapped-doc-head">
            <strong>Edit off-chain metadata</strong>
          </div>
          <p className="sub" style={{ margin: "6px 0 10px" }}>
            Only the label and notes are editable. The tx hash, block, contract address and anchored
            document hash are immutable on-chain state.
          </p>
          <label>Label</label>
          <input data-testid="edit-label" value={editLabel} onChange={(e) => setEditLabel(e.target.value)} />
          <label>Notes</label>
          <input data-testid="edit-notes" value={editNotes} onChange={(e) => setEditNotes(e.target.value)} />
          <div style={{ marginTop: 10 }}>
            <button data-testid="edit-save" disabled={busy === editRoot} onClick={() => void saveEdit()}>
              Save
            </button>{" "}
            <button onClick={() => setEditRoot(null)}>Cancel</button>
          </div>
        </div>
      )}
    </div>
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
        <Route path="/verify" element={<VerifyPage health={health} />} />
        <Route path="/records" element={<RecordsPage />} />
      </Routes>
      <p className="muted">
        chainId {health?.chainId ?? "?"} · signer {health?.signer ?? "none"} ·{" "}
        {health?.canSign ? "can anchor on-chain" : "read-only (no signer)"}
      </p>
    </div>
  );
}

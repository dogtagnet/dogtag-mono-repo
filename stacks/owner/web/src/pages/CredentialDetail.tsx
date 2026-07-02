import { useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { decodeFields, summarize } from "../lib/credential";
import { isRootAnchored } from "../lib/chain";
import { credentialStore } from "../lib/store";
import { useCredentials } from "../lib/hooks";
import { categoryIcon, categoryVar } from "../lib/ui";
import type { OwnerWallet } from "../lib/wallet";

type OnChain = "checking" | "valid" | "invalid" | "unknown";

export function CredentialDetail({ wallet }: { wallet: OwnerWallet | null }) {
  const { id = "" } = useParams();
  const decodedId = decodeURIComponent(id);
  const credentials = useCredentials();
  const credential = credentials.find((c) => c.id === decodedId);
  const navigate = useNavigate();
  const [onchain, setOnchain] = useState<OnChain>("checking");

  const doc = credential?.wrappedDoc;

  useEffect(() => {
    if (!doc) return;
    let live = true;
    setOnchain("checking");
    isRootAnchored(doc.issuer.documentStore, doc.signature.merkleRoot)
      .then((ok) => live && setOnchain(ok ? "valid" : "invalid"))
      .catch(() => live && setOnchain("unknown"));
    return () => {
      live = false;
    };
  }, [doc?.signature.merkleRoot, doc?.issuer.documentStore]);

  if (!credential || !doc) {
    return (
      <div className="card">
        <h2>Credential not found</h2>
        <p className="sub">It may have been removed from this wallet.</p>
        <Link to="/wallet" className="btn">
          Back to wallet
        </Link>
      </div>
    );
  }

  const s = summarize(doc);
  const fields = decodeFields(doc);

  function remove() {
    credentialStore.remove(decodedId);
    navigate("/wallet");
  }

  return (
    <div style={{ ["--cat" as string]: categoryVar(s.category) }}>
      <Link to="/wallet" className="back-link">
        ← My wallet
      </Link>

      <div className="hero">
        <div className="cred-avatar" aria-hidden>
          {categoryIcon(s.category)}
        </div>
        <div>
          <h1 data-testid="detail-name">{s.petName ?? s.recordType}</h1>
          <div className="cred-type">
            {s.recordType} · issued by {s.issuerName}
          </div>
        </div>
      </div>

      <div className="card">
        <div className="cred-meta" style={{ marginTop: 0 }}>
          <span className={`badge ${s.integrity === "VALID" ? "ok" : "bad"}`} data-testid="detail-integrity">
            {s.integrity === "VALID" ? "✓ Integrity intact" : "✗ Integrity failed"}
          </span>
          <span
            className={`badge ${onchain === "valid" ? "ok" : onchain === "invalid" ? "bad" : "warn"}`}
            data-testid="detail-onchain"
          >
            {onchain === "checking"
              ? "⏳ Checking on-chain…"
              : onchain === "valid"
                ? "✓ Anchored on-chain"
                : onchain === "invalid"
                  ? "✗ Not anchored"
                  : "· On-chain status unavailable"}
          </span>
        </div>

        <h2 style={{ marginTop: 18 }}>Credential fields</h2>
        <p className="sub">Exactly the values hashed into this credential's on-chain Merkle root.</p>
        <div className="field-grid" data-testid="detail-fields">
          {fields.map((f) => (
            <div className="field-cell" key={f.keyPath}>
              <div className="k">{f.label}</div>
              <div className="v">{f.value}</div>
            </div>
          ))}
        </div>

        <h2 style={{ marginTop: 20 }}>Cryptographic identity</h2>
        <div>
          <div className="kv">
            <span className="k">Merkle root</span>
            <span className="v">{s.credentialRoot}</span>
          </div>
          <div className="kv">
            <span className="k">Issuer clone</span>
            <span className="v">{s.documentStore}</span>
          </div>
          <div className="kv">
            <span className="k">Issuer domain</span>
            <span className="v">{s.issuerDomain || "—"}</span>
          </div>
        </div>

        <div className="btn-row">
          <Link
            to={`/present?id=${encodeURIComponent(credential.id)}`}
            className="btn"
            data-testid="detail-present"
            aria-disabled={!wallet}
          >
            Present a proof →
          </Link>
          <button type="button" className="btn secondary" data-testid="detail-remove" onClick={remove}>
            Remove
          </button>
        </div>
      </div>
    </div>
  );
}

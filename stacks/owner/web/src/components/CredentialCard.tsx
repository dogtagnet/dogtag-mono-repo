import { Link } from "react-router-dom";
import { summarize } from "../lib/credential";
import { categoryIcon, categoryVar } from "../lib/ui";
import type { StoredCredential } from "../lib/store";

/** A single held credential, rendered as a tappable wallet card. */
export function CredentialCard({ credential }: { credential: StoredCredential }) {
  const s = summarize(credential.wrappedDoc);
  const held = s.integrity === "VALID";
  return (
    <Link
      to={`/credential/${encodeURIComponent(credential.id)}`}
      className="cred-card"
      style={{ ["--cat" as string]: categoryVar(s.category) }}
      data-testid="cred-card"
      data-record-type={s.recordType}
    >
      <div className="cred-top">
        <div className="cred-avatar" aria-hidden>
          {categoryIcon(s.category)}
        </div>
        <div className="cred-main">
          <div className="cred-name" data-testid="cred-name">
            {s.petName ?? s.recordType}
          </div>
          <div className="cred-type">
            {s.recordType} · {s.issuerName}
          </div>
        </div>
        <span className="cat-tag">{s.category}</span>
      </div>
      <div className="cred-meta">
        {s.dogTagId && (
          <span>
            <b>Dog tag</b> #{s.dogTagId}
          </span>
        )}
        {s.validUntil && (
          <span>
            <b>Valid until</b> {s.validUntil}
          </span>
        )}
        <span className={`badge ${held ? "ok" : "bad"}`} data-testid="cred-integrity">
          {held ? "✓ Intact" : "✗ Tampered"}
        </span>
      </div>
    </Link>
  );
}

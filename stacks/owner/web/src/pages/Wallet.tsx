import { Link } from "react-router-dom";
import { CredentialCard } from "../components/CredentialCard";
import { useCredentials } from "../lib/hooks";

/** The wallet home — every credential the owner holds, grouped nowhere fancy: newest first. */
export function Wallet() {
  const credentials = useCredentials();

  return (
    <>
      <div className="section-title">
        <h1>My credentials</h1>
        <span className="count-pill" data-testid="cred-count">
          {credentials.length} held
        </span>
      </div>

      {credentials.length === 0 ? (
        <div className="empty" data-testid="empty-wallet">
          <div className="big" aria-hidden>
            🐾
          </div>
          <h3>Your wallet is empty</h3>
          <p>
            When a vet, groomer, or government authority issues your pet a credential, add it here to
            hold it and present zero-knowledge proofs.
          </p>
          <Link to="/receive" className="btn" data-testid="empty-receive">
            Receive a credential
          </Link>
        </div>
      ) : (
        <div className="cred-list" data-testid="cred-list">
          {credentials.map((c) => (
            <CredentialCard key={c.id} credential={c} />
          ))}
        </div>
      )}
    </>
  );
}

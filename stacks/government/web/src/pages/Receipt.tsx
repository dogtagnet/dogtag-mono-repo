import { Button, QrCode } from "@dogtag/ui";
import { Printer } from "lucide-react";
import { useEffect, useState } from "react";
import { Link, useParams } from "react-router-dom";
import {
  API_BASE,
  apiGet,
  publicReceiptUrl,
  statusAccent,
  type GovRecord,
  type ReceiptStatus,
} from "../lib/api";

type Subject = Record<string, unknown>;

/** Read a nested `credentialSubject` value by dotted path (e.g. "importer.firstName"). */
function pick(subject: Subject | null | undefined, path: string): unknown {
  if (!subject) return undefined;
  return path.split(".").reduce<unknown>((acc, key) => {
    if (acc && typeof acc === "object") return (acc as Record<string, unknown>)[key];
    return undefined;
  }, subject);
}

/** Humanize a snake_case enum value ("service_animal" → "Service Animal"); leave codes as-is. */
function humanize(v: unknown): string {
  if (v == null || v === "") return "";
  if (typeof v === "boolean") return v ? "Yes" : "No";
  const s = String(v);
  if (!s.includes("_")) return s;
  return s
    .split("_")
    .map((w) => (w ? w[0]!.toUpperCase() + w.slice(1) : w))
    .join(" ");
}

interface Row {
  label: string;
  value: string;
}

/** A CDC-anatomy section: a dark-slate header bar + a bordered label/value table. Rows whose value
 *  is empty are dropped so the receipt stays tight (matches the CDC form's blank-row omission). */
function Section({ title, rows }: { title: string; rows: Row[] }) {
  const present = rows.filter((r) => r.value !== "" && r.value != null);
  if (present.length === 0) return null;
  return (
    <div>
      <div className="receipt-section-bar">{title}</div>
      <table className="receipt-table">
        <tbody>
          {present.map((r) => (
            <tr key={r.label}>
              <td className="r-label">{r.label}</td>
              <td className="r-value">{r.value}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function s(v: unknown): string {
  return v == null ? "" : String(v);
}

export function Receipt() {
  const { root = "" } = useParams();
  const [record, setRecord] = useState<GovRecord | null>(null);
  const [status, setStatus] = useState<ReceiptStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;
    async function load() {
      setLoading(true);
      setError(null);
      try {
        // Authenticated custodial record (carries the Section A/B/C PII subject).
        const rec = await apiGet(`/v1/records/${root}`);
        if (cancelled) return;
        if (rec?.error || !rec?.root) {
          setError(rec?.error || "credential not found");
          setRecord(null);
          return;
        }
        setRecord(rec as GovRecord);
        // The LIVE on-chain status + authoritative issuance date (public, PII-free surface).
        if (rec.receiptId) {
          const st = await fetch(`${API_BASE}/v1/receipts/${rec.receiptId}/status`).then((r) => r.json());
          if (!cancelled && !st?.error) setStatus(st as ReceiptStatus);
        }
      } catch (e) {
        if (!cancelled) setError(String(e));
      } finally {
        if (!cancelled) setLoading(false);
      }
    }
    void load();
    return () => {
      cancelled = true;
    };
  }, [root]);

  if (loading) return <p className="text-sm text-muted">Loading receipt…</p>;
  if (error || !record)
    return (
      <div className="space-y-4">
        <p className="text-sm text-danger">Could not load receipt: {error ?? "unknown error"}</p>
        <Button asChild variant="outline" size="sm">
          <Link to="/records">← Back to records</Link>
        </Button>
      </div>
    );

  const subject = (record.subject as Subject) ?? {};
  const wrapped = record.wrappedDoc as
    | { issuer?: { name?: string }; privacy?: { obfuscated?: unknown[] } }
    | undefined;

  const effective = status?.effectiveStatus || record.effectiveStatus || record.status.toUpperCase();
  const accent = statusAccent(effective);
  const isTravel = record.recordType === "TRAVEL_CLEARANCE";

  const authorityName =
    wrapped?.issuer?.name || s(subject.endorsingAuthority) || "Competent Authority";
  const receiptId = record.receiptId || "—";

  const issuanceDate =
    status?.issuanceDate ||
    (record.createdAt ? new Date(record.createdAt * 1000).toISOString().slice(0, 10) : "—");
  const validUntil = record.validUntil || s(pick(subject, "validity.validUntil")) || "—";
  const multipleEntries = pick(subject, "validity.multipleEntries") === true;
  const departureBinding = s(pick(subject, "validity.countryOfDepartureBinding"));

  const obfuscatedCount = Array.isArray(wrapped?.privacy?.obfuscated)
    ? wrapped!.privacy!.obfuscated!.length
    : 0;

  // Section A — person importing the animal (PII, custodial view).
  const sectionA: Row[] = [
    { label: "First Name", value: s(pick(subject, "importer.firstName")) },
    { label: "Middle Name/Initial", value: s(pick(subject, "importer.middleName")) },
    { label: "Last Name", value: s(pick(subject, "importer.lastName")) },
    { label: "The person listed above is the", value: humanize(pick(subject, "importer.role")) },
    { label: "Identification Type", value: humanize(pick(subject, "importer.idType")) },
    { label: "ID Jurisdiction (state/country)", value: s(pick(subject, "importer.idJurisdiction")) },
    { label: "Identification No.", value: s(pick(subject, "importer.idNumber")) },
    { label: "Importer Date of Birth", value: s(pick(subject, "importer.dateOfBirth")) },
    { label: "Email", value: s(pick(subject, "importer.email")) },
    { label: "Phone Number", value: s(pick(subject, "importer.phone")) },
    { label: "Consignee/Additional Owner", value: s(pick(subject, "consignee.fullName")) },
    { label: "Consignee Email", value: s(pick(subject, "consignee.email")) },
    { label: "Consignee Identification Type", value: humanize(pick(subject, "consignee.idType")) },
  ];

  // Section B — animal information. CDC's "Male Neutered" = sex + neutered combined.
  const sex = s(pick(subject, "animal.sex"));
  const neutered = pick(subject, "animal.neutered") === true;
  const sexValue = sex ? `${sex[0]!.toUpperCase()}${sex.slice(1)}${neutered ? " Neutered" : ""}` : "";
  const sectionB: Row[] = [
    { label: "Animal Name", value: s(pick(subject, "animal.name")) },
    { label: "Age - Year(s)", value: s(pick(subject, "animal.ageYears")) },
    { label: "Age - Month(s)", value: s(pick(subject, "animal.ageMonths")) },
    { label: "Sex", value: sexValue },
    { label: "Dog Breed", value: s(pick(subject, "animal.breed")) },
    { label: "Color/Markings", value: s(pick(subject, "animal.colorMarkings")) },
    { label: "Microchip Number", value: s(pick(subject, "animal.microchipNumber")) },
    { label: "Importation Purpose", value: humanize(pick(subject, "animal.importationPurpose")) },
  ];

  // Section C — travel information.
  const sectionC: Row[] = [
    { label: "Travel Type", value: humanize(pick(subject, "travel.travelType")) },
    { label: "Country or Area of Departure", value: s(pick(subject, "travel.countryOfDeparture")) },
    { label: "Date of Arrival", value: s(pick(subject, "travel.dateOfArrival")) },
    { label: "Port of Entry", value: s(pick(subject, "travel.portOfEntry")) },
    { label: "Carrier / Flight", value: s(pick(subject, "travel.carrierOrFlight")) },
  ];

  // EU_HEALTH_CERT — the Annex-IV clinical block (flat subject).
  const healthRows: Row[] = [
    { label: "Species", value: s(subject.species) },
    { label: "Microchip Number", value: s(subject.microchipNumber) },
    { label: "Rabies Vaccination Date", value: s(subject.rabiesVaccinationDate) },
    { label: "Rabies Valid Until", value: s(subject.rabiesValidUntil) },
    { label: "Examining Veterinarian", value: s(subject.examiningVeterinarian) },
    { label: "Clinical Health Status", value: humanize(subject.clinicalHealthStatus) },
    { label: "Examination Date", value: s(subject.examinationDate) },
  ];

  return (
    <div className="space-y-4">
      {/* controls — never printed */}
      <div className="no-print flex items-center justify-between gap-3">
        <Button asChild variant="ghost" size="sm">
          <Link to="/records">← Back to records</Link>
        </Button>
        <Button size="sm" onClick={() => window.print()}>
          <Printer className="h-4 w-4" /> Print receipt
        </Button>
      </div>

      <div className="receipt-sheet mx-auto max-w-3xl" data-testid="receipt-sheet">
        {/* letterhead */}
        <div className="receipt-letterhead">
          <span className="receipt-seal" aria-hidden>
            🏛
          </span>
          <div>
            <div style={{ fontWeight: 700, fontSize: "1.05rem", color: "#0f172a" }}>{authorityName}</div>
            <div style={{ fontSize: "0.8rem", color: "#475569" }}>
              Pet Travel Clearance · authority-endorsed credential
            </div>
          </div>
        </div>

        <div className="r-body">
          {/* title + status */}
          <div className="flex flex-wrap items-center justify-between gap-3">
            <h2 style={{ fontSize: "1.05rem", fontWeight: 600, color: "#334155", margin: 0 }}>
              {isTravel ? "Pet Travel Clearance Receipt" : "Pet Health Certificate"}
            </h2>
            <span
              className="receipt-status-chip"
              style={{ background: accent }}
              data-testid="receipt-status"
            >
              ● {effective}
            </span>
          </div>

          {/* receipt identity block */}
          <div style={{ marginTop: "1.1rem" }}>
            <div
              style={{ fontSize: "1.25rem", fontWeight: 700, color: "#0f172a" }}
              data-testid="receipt-id"
            >
              Receipt ID: {receiptId}
            </div>
            <table className="receipt-table" style={{ marginTop: "0.75rem" }}>
              <tbody>
                <tr>
                  <td className="r-label">Date of issuance</td>
                  <td className="r-value">{issuanceDate}</td>
                </tr>
                <tr>
                  <td className="r-label">
                    {multipleEntries ? "Valid for multiple entries until" : "Valid until"}
                  </td>
                  <td className="r-value">{validUntil}</td>
                </tr>
                <tr>
                  <td className="r-label">Dog Tag ID (SBT)</td>
                  <td className="r-value">#{s(subject.dogTagId) || record.dogTagId}</td>
                </tr>
              </tbody>
            </table>
          </div>

          {/* legal preamble */}
          <p style={{ marginTop: "1.1rem", fontSize: "0.82rem", color: "#475569", lineHeight: 1.5 }}>
            This receipt is valid for the animal listed for the validity window shown above
            {departureBinding ? `, for entry from the listed country of departure (${departureBinding})` : ""}.
            If the animal travels via a different or high-risk country, a new clearance may be required.{" "}
            <strong style={{ color: "#0f172a" }}>
              You must show this receipt (printed or on your phone) to airline staff and port-of-entry
              officials.
            </strong>{" "}
            The authority reserves the right to request additional supporting documentation on arrival.
          </p>

          {obfuscatedCount > 0 && (
            <p style={{ marginTop: "0.75rem", fontSize: "0.78rem", color: "#b45309" }}>
              Note: {obfuscatedCount} field(s) were withheld (obfuscated) by the holder in this
              presentation copy; withheld leaves still verify against the on-chain root.
            </p>
          )}

          {/* sectioned content */}
          {isTravel ? (
            <>
              <Section title="Section A - Person Importing the Animal" rows={sectionA} />
              <Section title="Section B - Animal Information" rows={sectionB} />
              <Section title="Section C - Travel Information" rows={sectionC} />
            </>
          ) : (
            <Section title="Health Certificate (Annex IV)" rows={healthRows} />
          )}

          {/* verification block — QR to the PII-free public status page + on-chain provenance */}
          <div className="receipt-section-bar">Verification</div>
          <div
            style={{
              border: "1px solid #e2e8f0",
              borderTop: "none",
              padding: "1rem",
              display: "flex",
              flexWrap: "wrap",
              gap: "1.25rem",
              alignItems: "flex-start",
            }}
          >
            {record.receiptId && (
              <span data-testid="receipt-qr">
                <QrCode value={publicReceiptUrl(record.receiptId)} size={132} />
              </span>
            )}
            <div style={{ flex: "1 1 16rem", minWidth: "14rem", fontSize: "0.8rem", color: "#475569" }}>
              <div style={{ fontWeight: 600, color: "#0f172a" }}>
                Scan to confirm live status on-chain
              </div>
              <div style={{ marginTop: "0.35rem", wordBreak: "break-all" }}>
                {record.receiptId ? publicReceiptUrl(record.receiptId) : "—"}
              </div>
              <div style={{ marginTop: "0.6rem" }}>
                <span style={{ color: "#64748b" }}>Credential root:</span>
                <div className="receipt-mono" style={{ color: "#0f172a" }}>
                  {record.root}
                </div>
              </div>
              <div style={{ marginTop: "0.5rem" }}>
                Anchored: ROAX{status ? "" : ""} · block {record.blockNumber ?? "?"} · issuer{" "}
                <span className="receipt-mono">{record.issuerAddr.slice(0, 12)}…</span>
              </div>
              {record.explorerUrl && (
                <div style={{ marginTop: "0.35rem" }}>
                  <a
                    href={record.explorerUrl}
                    target="_blank"
                    rel="noreferrer"
                    style={{ color: "#2563eb" }}
                  >
                    issue tx ↗
                  </a>
                  {record.revokeExplorerUrl && (
                    <>
                      {" · "}
                      <a
                        href={record.revokeExplorerUrl}
                        target="_blank"
                        rel="noreferrer"
                        style={{ color: "#dc2626" }}
                      >
                        revoke tx ↗
                      </a>
                    </>
                  )}
                </div>
              )}
              <div style={{ marginTop: "0.6rem", fontSize: "0.72rem", color: "#94a3b8" }}>
                The public status page shows NO personal data — only the live VALID / EXPIRED /
                REVOKED verdict and provenance.
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}

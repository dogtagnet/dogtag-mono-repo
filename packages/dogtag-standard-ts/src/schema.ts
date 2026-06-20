// DogTag credential schema validator (impl §1.6 full field set + §11.5 corrected
// conditional/jurisdiction rules). Operates on a pre-wrap, plain-JSON credential object
// (ordinary string/number/array/object fields — NOT the typed-scalar-leaf form).
// Returns the credential unchanged on success; throws SchemaError listing ALL violations.

export const DOGTAG_CONTEXT_URI = "https://dogtag.io/credentials/v1";

export class SchemaError extends Error {
  readonly violations: string[];
  constructor(violations: string[]) {
    super(`schema validation failed:\n  - ${violations.join("\n  - ")}`);
    this.name = "SchemaError";
    this.violations = violations;
  }
}

type Json = unknown;
type Obj = Record<string, Json>;

function isObject(v: Json): v is Obj {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}
function isString(v: Json): v is string {
  return typeof v === "string";
}
function isArray(v: Json): v is Json[] {
  return Array.isArray(v);
}
function includes(arr: Json, val: string): boolean {
  return isArray(arr) && arr.some((x) => x === val);
}

// --- dependency-light date math: ISO "YYYY-MM-DD" (optionally with a time suffix) ---
// civil-from/to-days (Howard Hinnant's algorithm), days since 1970-01-01.

function parseIsoDate(s: string): number | null {
  // accept "YYYY-MM-DD" optionally followed by "T..." or " ..."
  const m = /^(\d{4})-(\d{2})-(\d{2})/.exec(s);
  if (!m) return null;
  const y = Number(m[1]);
  const mo = Number(m[2]);
  const d = Number(m[3]);
  if (mo < 1 || mo > 12 || d < 1 || d > 31) return null;
  return daysFromCivil(y, mo, d);
}

function daysFromCivil(y: number, m: number, d: number): number {
  // y/m/d -> days since 1970-01-01 (proleptic Gregorian).
  const yy = m <= 2 ? y - 1 : y;
  const era = Math.floor((yy >= 0 ? yy : yy - 399) / 400);
  const yoe = yy - era * 400; // [0, 399]
  const doy = Math.floor((153 * (m > 2 ? m - 3 : m + 9) + 2) / 5) + d - 1; // [0, 365]
  const doe = yoe * 365 + Math.floor(yoe / 4) - Math.floor(yoe / 100) + doy; // [0, 146096]
  return era * 146097 + doe - 719468;
}

function isoDate(a: string): number | null {
  return parseIsoDate(a);
}

/** Compare two decimal strings (non-negative) numerically without float parsing. */
function decimalGte(a: string, b: string): boolean {
  return compareDecimal(a, b) >= 0;
}
function compareDecimal(a: string, b: string): number {
  const pa = splitDecimal(a);
  const pb = splitDecimal(b);
  if (pa === null || pb === null) return NaN as unknown as number;
  // integer part compare (strip leading zeros)
  const ia = stripLeadingZeros(pa.int);
  const ib = stripLeadingZeros(pb.int);
  if (ia.length !== ib.length) return ia.length < ib.length ? -1 : 1;
  if (ia !== ib) return ia < ib ? -1 : 1;
  // fractional part compare, right-pad
  const len = Math.max(pa.frac.length, pb.frac.length);
  const fa = pa.frac.padEnd(len, "0");
  const fb = pb.frac.padEnd(len, "0");
  if (fa === fb) return 0;
  return fa < fb ? -1 : 1;
}
function splitDecimal(s: string): { int: string; frac: string } | null {
  if (!/^\d+(\.\d+)?$/.test(s)) return null;
  const [int = "0", frac = ""] = s.split(".");
  return { int, frac };
}
function stripLeadingZeros(s: string): string {
  const t = s.replace(/^0+/, "");
  return t === "" ? "0" : t;
}
function isDecimalString(v: Json): v is string {
  return isString(v) && /^\d+(\.\d+)?$/.test(v);
}

/**
 * Validate a credential object. Returns the credential on success;
 * throws {@link SchemaError} (with all violations) on failure.
 */
export function validateSchema<T extends Obj>(credential: T): T {
  const errs: string[] = [];
  const c = credential as Obj;
  const push = (msg: string) => errs.push(msg);
  const reqPresent = (v: Json, path: string) => {
    if (v === undefined || v === null) push(`${path} is required`);
  };

  // --- VC 2.0 envelope ---
  const ctx = c["@context"];
  if (!isArray(ctx)) {
    push(`@context must be an array`);
  } else {
    if (ctx[0] !== "https://www.w3.org/ns/credentials/v2") {
      push(`@context[0] must be "https://www.w3.org/ns/credentials/v2"`);
    }
    if (!includes(ctx, DOGTAG_CONTEXT_URI)) {
      push(`@context must include DogTag context URI "${DOGTAG_CONTEXT_URI}"`);
    }
  }
  if (!isArray(c.type)) {
    push(`type must be an array`);
  } else if (!includes(c.type, "VerifiableCredential")) {
    push(`type must include "VerifiableCredential"`);
  }
  reqPresent(c.id, "id");
  reqPresent(c.issuer, "issuer");
  reqPresent(c.validFrom, "validFrom");
  reqPresent(c.credentialSubject, "credentialSubject");
  reqPresent(c.credentialSchema, "credentialSchema");
  reqPresent(c.credentialStatus, "credentialStatus");
  if (c.description !== undefined && !isString(c.description)) {
    push(`description must be a string`);
  }
  const subject: Obj = isObject(c.credentialSubject) ? c.credentialSubject : {};
  reqPresent(subject.dogTagId, "credentialSubject.dogTagId");

  // --- legal/trust meta on every credential ---
  reqPresent(c.attestationType, "attestationType");
  const STT = ["accredited_authority", "licensed_vet", "self_attested"];
  if (!isString(c.signatureTrustTier) || !STT.includes(c.signatureTrustTier)) {
    push(`signatureTrustTier must be one of {${STT.join(", ")}}`);
  }
  if (c.legalEffect !== "evidentiary") {
    push(`legalEffect must == "evidentiary"`);
  }
  reqPresent(c.legalBasisVersion, "legalBasisVersion");
  reqPresent(c.jurisdiction, "jurisdiction");

  const type = c.type;
  const isRabies = includes(type, "RabiesVaccinationCertificate");

  // --- microchip = OBJECT, never float/bare number ---
  const microchip = subject.microchip;
  const needsChip =
    isRabies || c.recordType === "EU_HEALTH_CERT" || c.cdcPath === "standard";
  if (microchip !== undefined || needsChip) {
    if (!isObject(microchip)) {
      push(`credentialSubject.microchip must be an object`);
    } else {
      const code = microchip.code;
      if (!isString(code)) {
        push(`credentialSubject.microchip.code must be a string`);
      } else if (!/^[0-9]{15}$/.test(code)) {
        push(`credentialSubject.microchip.code must match ^[0-9]{15}$`);
      }
      const STD = ["ISO_11784_11785", "OTHER"];
      if (!isString(microchip.standard) || !STD.includes(microchip.standard)) {
        push(`credentialSubject.microchip.standard must be one of {${STD.join(", ")}}`);
      }
      reqPresent(microchip.implantDate, "credentialSubject.microchip.implantDate");
    }
  }

  // --- DOG_PROFILE ---
  if (c.recordType === "DOG_PROFILE") {
    reqPresent(subject.species, "credentialSubject.species");
    reqPresent(subject.breedVbo, "credentialSubject.breedVbo");
    reqPresent(subject.breedLabel, "credentialSubject.breedLabel");
    const SEX = ["male", "female"];
    if (!isString(subject.sex) || !SEX.includes(subject.sex)) {
      push(`credentialSubject.sex must be one of {${SEX.join(", ")}}`);
    }
    const NEU = ["intact", "neutered", "spayed"];
    if (!isString(subject.neuterStatus) || !NEU.includes(subject.neuterStatus)) {
      push(`credentialSubject.neuterStatus must be one of {${NEU.join(", ")}}`);
    }
    reqPresent(subject.dateOfBirth, "credentialSubject.dateOfBirth");
    // owner's official identity — OBJECT with three string sub-fields (keys must be present;
    // empty strings allowed for non-admin mint paths).
    const ownerIdentity = subject.ownerIdentity;
    if (!isObject(ownerIdentity)) {
      push(`credentialSubject.ownerIdentity must be an object`);
    } else {
      for (const f of ["countryOfIdentification", "identification", "name"]) {
        if (!isString(ownerIdentity[f])) {
          push(`credentialSubject.ownerIdentity.${f} must be a string`);
        }
      }
    }
    const wh = subject.weightHistory;
    if (wh !== undefined) {
      if (!isArray(wh)) {
        push(`credentialSubject.weightHistory must be an array`);
      } else {
        wh.forEach((w, i) => {
          const p = `credentialSubject.weightHistory[${i}]`;
          if (!isObject(w)) {
            push(`${p} must be an object`);
            return;
          }
          const U = ["kg", "lb"];
          if (!isString(w.unit) || !U.includes(w.unit)) {
            push(`${p}.unit must be one of {${U.join(", ")}}`);
          }
          if (!isDecimalString(w.value)) {
            push(`${p}.value must be a decimal string`);
          }
          reqPresent(w.measuredOn, `${p}.measuredOn`);
        });
      }
    }
  }

  // --- VACCINATION (RabiesVaccinationCertificate) ---
  if (isRabies) {
    reqPresent(c.vaccineProductCode, "vaccineProductCode");
    reqPresent(c.vaccineProductName, "vaccineProductName");
    reqPresent(c.vaccineManufacturer, "vaccineManufacturer");
    reqPresent(c.batchLotNumber, "batchLotNumber");
    reqPresent(c.vaccinationDate, "vaccinationDate");
    reqPresent(c.validFrom, "validFrom");
    reqPresent(c.validUntil, "validUntil");
    reqPresent(c.nextDueDate, "nextDueDate");
    reqPresent(c.authorizedVet, "authorizedVet");
    const SERIES = ["primary", "booster"];
    if (!isString(c.series) || !SERIES.includes(c.series)) {
      push(`series must be one of {${SERIES.join(", ")}}`);
    }

    const vaccDate = isString(c.vaccinationDate) ? isoDate(c.vaccinationDate) : null;

    // microchip.implantDate <= vaccinationDate
    if (isObject(microchip) && isString(microchip.implantDate) && isString(c.vaccinationDate)) {
      const impl = isoDate(microchip.implantDate);
      if (impl !== null && vaccDate !== null && impl > vaccDate) {
        push(`microchip.implantDate must be <= vaccinationDate`);
      }
    }

    // animal age at vaccination >= 12 weeks (from dateOfBirth)
    if (isString(subject.dateOfBirth) && vaccDate !== null) {
      const dob = isoDate(subject.dateOfBirth);
      if (dob !== null && vaccDate - dob < 12 * 7) {
        push(`animal age at vaccination must be >= 12 weeks`);
      }
    }

    // primary series: validFrom == vaccinationDate + 21 days
    if (c.series === "primary" && vaccDate !== null && isString(c.validFrom)) {
      const vf = isoDate(c.validFrom);
      if (vf === null || vf !== vaccDate + 21) {
        push(`primary series: validFrom must == vaccinationDate + 21 days`);
      }
    }

    // titer
    if (subject.titer !== undefined || c.titer !== undefined) {
      const titer = (c.titer !== undefined ? c.titer : subject.titer) as Json;
      if (!isObject(titer)) {
        push(`titer must be an object`);
      } else {
        if (!isDecimalString(titer.resultIUml)) {
          push(`titer.resultIUml must be a decimal string`);
        } else if (!decimalGte(titer.resultIUml, "0.5")) {
          push(`titer.resultIUml must be >= "0.5"`);
        }
        if (isString(titer.sampledAt) && vaccDate !== null) {
          const sa = isoDate(titer.sampledAt);
          if (sa === null || sa < vaccDate + 30) {
            push(`titer.sampledAt must be >= vaccinationDate + 30 days`);
          }
        } else {
          reqPresent(titer.sampledAt, "titer.sampledAt");
        }
      }
    }
  }

  // --- SERVICE_ATTESTATION ---
  if (c.recordType === "SERVICE_ATTESTATION") {
    const AT = ["service_dog", "emotional_support", "none"];
    if (!isString(c.assistanceType) || !AT.includes(c.assistanceType)) {
      push(`assistanceType must be one of {${AT.join(", ")}}`);
    }
    const ITT = ["adi_accredited", "licensed_pro", "handler_self_attestation", "unverified_registry"];
    if (!isString(c.issuerTrustTier) || !ITT.includes(c.issuerTrustTier)) {
      push(`issuerTrustTier must be one of {${ITT.join(", ")}}`);
    }
    reqPresent(c.taskDescription, "taskDescription");
    const LC = ["ADA", "ACAA", "FHA"];
    const legalContext = c.legalContext;
    if (legalContext !== undefined) {
      if (!isArray(legalContext)) {
        push(`legalContext must be an array`);
      } else {
        legalContext.forEach((ctx2, i) => {
          if (!isString(ctx2) || !LC.includes(ctx2)) {
            push(`legalContext[${i}] must be one of {${LC.join(", ")}}`);
          }
        });
      }
    }
    if (c.storage !== "off_chain") {
      push(`storage must == "off_chain" (Art.9 special-category, never on-chain)`);
    }
  }

  // --- jurisdiction-specific ---
  if (c.recordType === "EU_HEALTH_CERT") {
    if (isString(c.validFrom) && isString(c.validUntilEntry)) {
      const vf = isoDate(c.validFrom);
      const vue = isoDate(c.validUntilEntry);
      if (vf === null || vue === null || vue !== vf + 10) {
        push(`EU_HEALTH_CERT: validUntilEntry must == validFrom + 10 days`);
      }
    } else {
      reqPresent(c.validUntilEntry, "validUntilEntry");
    }
    // onwardValid <= entry + 4 months (entry == validUntilEntry)
    if (isString(c.onwardValid) && isString(c.validUntilEntry)) {
      const ov = isoDate(c.onwardValid);
      const entry = isoDate(c.validUntilEntry);
      if (ov !== null && entry !== null && ov > addMonths(entry, 4)) {
        push(`EU_HEALTH_CERT: onwardValid must be <= entry + 4 months`);
      }
    }
    // echinococcus: 24h <= treatmentBeforeEntry <= 120h
    if (c.echinococcusRequired === true) {
      const t = c.treatmentBeforeEntry;
      if (typeof t !== "number" || t < 24 || t > 120) {
        push(`EU_HEALTH_CERT: echinococcus treatmentBeforeEntry must be within [24h, 120h]`);
      }
    }
  }
  if (c.recordType === "CDC_IMPORT_FORM") {
    const a = c.ageMonthsAtEntry;
    if (typeof a !== "number" || a < 6) {
      push(`CDC_IMPORT_FORM: ageMonthsAtEntry must be >= 6`);
    }
  }
  if (includes(type, "DOT")) {
    // handler attestation, not vet — set trustLevel = SELF_ATTESTED.
    c.trustLevel = "SELF_ATTESTED";
  }

  if (errs.length > 0) {
    throw new SchemaError(errs);
  }
  return credential;
}

/** Add `months` to a days-since-epoch value via civil arithmetic (entry + 4mo). */
function addMonths(days: number, months: number): number {
  const [y, m, d] = civilFromDays(days);
  let nm = m + months;
  let ny = y + Math.floor((nm - 1) / 12);
  nm = ((nm - 1) % 12) + 1;
  // clamp day to month length
  const dim = daysInMonth(ny, nm);
  const nd = Math.min(d, dim);
  return daysFromCivil(ny, nm, nd);
}
function civilFromDays(z0: number): [number, number, number] {
  const z = z0 + 719468;
  const era = Math.floor((z >= 0 ? z : z - 146096) / 146097);
  const doe = z - era * 146097; // [0, 146096]
  const yoe = Math.floor((doe - Math.floor(doe / 1460) + Math.floor(doe / 36524) - Math.floor(doe / 146096)) / 365);
  const y = yoe + era * 400;
  const doy = doe - (365 * yoe + Math.floor(yoe / 4) - Math.floor(yoe / 100));
  const mp = Math.floor((5 * doy + 2) / 153);
  const d = doy - Math.floor((153 * mp + 2) / 5) + 1;
  const m = mp < 10 ? mp + 3 : mp - 9;
  return [m <= 2 ? y + 1 : y, m, d];
}
function daysInMonth(y: number, m: number): number {
  const leap = (y % 4 === 0 && y % 100 !== 0) || y % 400 === 0;
  return [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][m - 1] ?? 30;
}

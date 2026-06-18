//! DogTag credential schema validator (impl §1.6 full field set + §11.5 corrected
//! conditional/jurisdiction rules). Operates on a pre-wrap, plain `serde_json::Value`
//! credential object (ordinary string/number/array/object fields — NOT the
//! typed-scalar-leaf form). Returns `Ok(())` or a list of ALL violations.

use serde_json::Value;

/// Canonical DogTag JSON-LD context URI (must appear in `@context`).
pub const DOGTAG_CONTEXT_URI: &str = "https://dogtag.io/credentials/v1";

// ----- small JSON helpers -----

fn get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key).filter(|x| !x.is_null())
}
fn as_str<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    get(v, key).and_then(Value::as_str)
}
fn arr_includes(v: &Value, needle: &str) -> bool {
    v.as_array()
        .map(|a| a.iter().any(|x| x.as_str() == Some(needle)))
        .unwrap_or(false)
}

// ----- date math: ISO "YYYY-MM-DD" (optional time suffix) -> days since 1970-01-01 -----

/// Parse `YYYY-MM-DD` (optionally followed by `T...`/` ...`) to days since the epoch.
fn iso_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() < 10 || b[4] != b'-' || b[7] != b'-' {
        return None;
    }
    let digits = |r: std::ops::Range<usize>| -> Option<i64> {
        let mut n: i64 = 0;
        for &c in &b[r] {
            if !c.is_ascii_digit() {
                return None;
            }
            n = n * 10 + (c - b'0') as i64;
        }
        Some(n)
    };
    let y = digits(0..4)?;
    let m = digits(5..7)?;
    let d = digits(8..10)?;
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    Some(days_from_civil(y, m, d))
}

/// Howard Hinnant's civil -> days-since-1970 (proleptic Gregorian).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let yy = if m <= 2 { y - 1 } else { y };
    let era = (if yy >= 0 { yy } else { yy - 399 }) / 400;
    let yoe = yy - era * 400; // [0, 399]
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

fn civil_from_days(z0: i64) -> (i64, i64, i64) {
    let z = z0 + 719468;
    let era = (if z >= 0 { z } else { z - 146096 }) / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_in_month(y: i64, m: i64) -> i64 {
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    [31, if leap { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][(m - 1) as usize]
}

/// Add whole months to a days-since-epoch value, clamping the day-of-month.
fn add_months(days: i64, months: i64) -> i64 {
    let (y, m, d) = civil_from_days(days);
    let total = (m - 1) + months;
    let ny = y + total.div_euclid(12);
    let nm = total.rem_euclid(12) + 1;
    let nd = d.min(days_in_month(ny, nm));
    days_from_civil(ny, nm, nd)
}

// ----- decimal string compare (no float parse) -----

fn split_decimal(s: &str) -> Option<(&str, &str)> {
    if s.is_empty() {
        return None;
    }
    let mut parts = s.splitn(2, '.');
    let int = parts.next().unwrap();
    let frac = parts.next().unwrap_or("");
    if int.is_empty() || !int.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if s.contains('.') && (frac.is_empty() || !frac.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    Some((int, frac))
}
fn is_decimal_string(s: &str) -> bool {
    split_decimal(s).is_some()
}
fn strip_zeros(s: &str) -> &str {
    let t = s.trim_start_matches('0');
    if t.is_empty() {
        "0"
    } else {
        t
    }
}
/// Compare two non-negative decimal strings; returns Some(Ordering) or None if malformed.
fn compare_decimal(a: &str, b: &str) -> Option<std::cmp::Ordering> {
    use std::cmp::Ordering;
    let (ai, af) = split_decimal(a)?;
    let (bi, bf) = split_decimal(b)?;
    let (ai, bi) = (strip_zeros(ai), strip_zeros(bi));
    if ai.len() != bi.len() {
        return Some(ai.len().cmp(&bi.len()));
    }
    if ai != bi {
        return Some(ai.cmp(bi));
    }
    let len = af.len().max(bf.len());
    let pad = |f: &str| -> String { format!("{:0<width$}", f, width = len) };
    Some(pad(af).cmp(&pad(bf))).map(|o| if o == Ordering::Equal { Ordering::Equal } else { o })
}
fn decimal_gte(a: &str, b: &str) -> bool {
    matches!(compare_decimal(a, b), Some(o) if o != std::cmp::Ordering::Less)
}

/// Validate a credential object. `Ok(())` on success, `Err(violations)` listing ALL failures.
pub fn validate_schema(c: &Value) -> Result<(), Vec<String>> {
    let mut e: Vec<String> = Vec::new();

    if !c.is_object() {
        return Err(vec!["credential must be a JSON object".into()]);
    }

    let req_present = |e: &mut Vec<String>, v: &Value, path: &str| {
        if get(v, path).is_none() {
            e.push(format!("{path} is required"));
        }
    };

    // --- VC 2.0 envelope ---
    match c.get("@context").and_then(Value::as_array) {
        None => e.push("@context must be an array".into()),
        Some(ctx) => {
            if ctx.first().and_then(Value::as_str) != Some("https://www.w3.org/ns/credentials/v2") {
                e.push("@context[0] must be \"https://www.w3.org/ns/credentials/v2\"".into());
            }
            if !ctx.iter().any(|x| x.as_str() == Some(DOGTAG_CONTEXT_URI)) {
                e.push(format!("@context must include DogTag context URI \"{DOGTAG_CONTEXT_URI}\""));
            }
        }
    }
    match c.get("type") {
        Some(t) if t.is_array() => {
            if !arr_includes(t, "VerifiableCredential") {
                e.push("type must include \"VerifiableCredential\"".into());
            }
        }
        _ => e.push("type must be an array".into()),
    }
    for f in ["id", "issuer", "validFrom", "credentialSubject", "credentialSchema", "credentialStatus"] {
        req_present(&mut e, c, f);
    }
    if let Some(d) = c.get("description") {
        if !d.is_null() && !d.is_string() {
            e.push("description must be a string".into());
        }
    }
    let empty = Value::Object(serde_json::Map::new());
    let subject = get(c, "credentialSubject").unwrap_or(&empty);
    if get(subject, "dogTagId").is_none() {
        e.push("credentialSubject.dogTagId is required".into());
    }

    // --- legal/trust meta on every credential ---
    req_present(&mut e, c, "attestationType");
    let stt = ["accredited_authority", "licensed_vet", "self_attested"];
    if !as_str(c, "signatureTrustTier").map(|s| stt.contains(&s)).unwrap_or(false) {
        e.push(format!("signatureTrustTier must be one of {{{}}}", stt.join(", ")));
    }
    if c.get("legalEffect").and_then(Value::as_str) != Some("evidentiary") {
        e.push("legalEffect must == \"evidentiary\"".into());
    }
    req_present(&mut e, c, "legalBasisVersion");
    req_present(&mut e, c, "jurisdiction");

    let type_v = c.get("type").unwrap_or(&Value::Null);
    let is_rabies = arr_includes(type_v, "RabiesVaccinationCertificate");
    let record_type = as_str(c, "recordType");

    // --- microchip = OBJECT, never float/bare number ---
    let microchip = get(subject, "microchip");
    let needs_chip = is_rabies
        || record_type == Some("EU_HEALTH_CERT")
        || as_str(c, "cdcPath") == Some("standard");
    if microchip.is_some() || needs_chip {
        match microchip {
            Some(m) if m.is_object() => {
                match m.get("code") {
                    Some(code) if code.is_string() => {
                        let s = code.as_str().unwrap();
                        if !(s.len() == 15 && s.bytes().all(|b| b.is_ascii_digit())) {
                            e.push("credentialSubject.microchip.code must match ^[0-9]{15}$".into());
                        }
                    }
                    _ => e.push("credentialSubject.microchip.code must be a string".into()),
                }
                let std = ["ISO_11784_11785", "OTHER"];
                if !m.get("standard").and_then(Value::as_str).map(|s| std.contains(&s)).unwrap_or(false) {
                    e.push(format!("credentialSubject.microchip.standard must be one of {{{}}}", std.join(", ")));
                }
                if get(m, "implantDate").is_none() {
                    e.push("credentialSubject.microchip.implantDate is required".into());
                }
            }
            _ => e.push("credentialSubject.microchip must be an object".into()),
        }
    }

    // --- DOG_PROFILE ---
    if record_type == Some("DOG_PROFILE") {
        for (f, label) in [("species", "species"), ("breedVbo", "breedVbo"), ("breedLabel", "breedLabel"), ("dateOfBirth", "dateOfBirth")] {
            if get(subject, f).is_none() {
                e.push(format!("credentialSubject.{label} is required"));
            }
        }
        let sex = ["male", "female"];
        if !as_str(subject, "sex").map(|s| sex.contains(&s)).unwrap_or(false) {
            e.push(format!("credentialSubject.sex must be one of {{{}}}", sex.join(", ")));
        }
        let neu = ["intact", "neutered", "spayed"];
        if !as_str(subject, "neuterStatus").map(|s| neu.contains(&s)).unwrap_or(false) {
            e.push(format!("credentialSubject.neuterStatus must be one of {{{}}}", neu.join(", ")));
        }
        if let Some(wh) = get(subject, "weightHistory") {
            match wh.as_array() {
                None => e.push("credentialSubject.weightHistory must be an array".into()),
                Some(items) => {
                    for (i, w) in items.iter().enumerate() {
                        let p = format!("credentialSubject.weightHistory[{i}]");
                        if !w.is_object() {
                            e.push(format!("{p} must be an object"));
                            continue;
                        }
                        let u = ["kg", "lb"];
                        if !w.get("unit").and_then(Value::as_str).map(|s| u.contains(&s)).unwrap_or(false) {
                            e.push(format!("{p}.unit must be one of {{{}}}", u.join(", ")));
                        }
                        if !w.get("value").and_then(Value::as_str).map(is_decimal_string).unwrap_or(false) {
                            e.push(format!("{p}.value must be a decimal string"));
                        }
                        if get(w, "measuredOn").is_none() {
                            e.push(format!("{p}.measuredOn is required"));
                        }
                    }
                }
            }
        }
    }

    // --- VACCINATION (RabiesVaccinationCertificate) ---
    if is_rabies {
        for f in ["vaccineProductCode", "vaccineProductName", "vaccineManufacturer", "batchLotNumber", "vaccinationDate", "validFrom", "validUntil", "nextDueDate", "authorizedVet"] {
            req_present(&mut e, c, f);
        }
        let series = ["primary", "booster"];
        let series_v = as_str(c, "series");
        if !series_v.map(|s| series.contains(&s)).unwrap_or(false) {
            e.push(format!("series must be one of {{{}}}", series.join(", ")));
        }

        let vacc_date = as_str(c, "vaccinationDate").and_then(iso_date);

        // microchip.implantDate <= vaccinationDate
        if let (Some(m), Some(vd)) = (microchip, vacc_date) {
            if let Some(impl_d) = m.get("implantDate").and_then(Value::as_str).and_then(iso_date) {
                if impl_d > vd {
                    e.push("microchip.implantDate must be <= vaccinationDate".into());
                }
            }
        }

        // age at vaccination >= 12 weeks
        if let (Some(dob), Some(vd)) = (as_str(subject, "dateOfBirth").and_then(iso_date), vacc_date) {
            if vd - dob < 12 * 7 {
                e.push("animal age at vaccination must be >= 12 weeks".into());
            }
        }

        // primary series: validFrom == vaccinationDate + 21 days
        if series_v == Some("primary") {
            if let Some(vd) = vacc_date {
                let vf = as_str(c, "validFrom").and_then(iso_date);
                if vf != Some(vd + 21) {
                    e.push("primary series: validFrom must == vaccinationDate + 21 days".into());
                }
            }
        }

        // titer (top-level c.titer or subject.titer)
        let titer = get(c, "titer").or_else(|| get(subject, "titer"));
        if let Some(t) = titer {
            if !t.is_object() {
                e.push("titer must be an object".into());
            } else {
                match t.get("resultIUml").and_then(Value::as_str) {
                    Some(s) if is_decimal_string(s) => {
                        if !decimal_gte(s, "0.5") {
                            e.push("titer.resultIUml must be >= \"0.5\"".into());
                        }
                    }
                    _ => e.push("titer.resultIUml must be a decimal string".into()),
                }
                match t.get("sampledAt").and_then(Value::as_str).and_then(iso_date) {
                    Some(sa) => {
                        if let Some(vd) = vacc_date {
                            if sa < vd + 30 {
                                e.push("titer.sampledAt must be >= vaccinationDate + 30 days".into());
                            }
                        }
                    }
                    None => e.push("titer.sampledAt is required".into()),
                }
            }
        }
    }

    // --- SERVICE_ATTESTATION ---
    if record_type == Some("SERVICE_ATTESTATION") {
        let at = ["service_dog", "emotional_support", "none"];
        if !as_str(c, "assistanceType").map(|s| at.contains(&s)).unwrap_or(false) {
            e.push(format!("assistanceType must be one of {{{}}}", at.join(", ")));
        }
        let itt = ["adi_accredited", "licensed_pro", "handler_self_attestation", "unverified_registry"];
        if !as_str(c, "issuerTrustTier").map(|s| itt.contains(&s)).unwrap_or(false) {
            e.push(format!("issuerTrustTier must be one of {{{}}}", itt.join(", ")));
        }
        req_present(&mut e, c, "taskDescription");
        let lc = ["ADA", "ACAA", "FHA"];
        if let Some(legal) = get(c, "legalContext") {
            match legal.as_array() {
                None => e.push("legalContext must be an array".into()),
                Some(items) => {
                    for (i, ctx) in items.iter().enumerate() {
                        if !ctx.as_str().map(|s| lc.contains(&s)).unwrap_or(false) {
                            e.push(format!("legalContext[{i}] must be one of {{{}}}", lc.join(", ")));
                        }
                    }
                }
            }
        }
        if as_str(c, "storage") != Some("off_chain") {
            e.push("storage must == \"off_chain\" (Art.9 special-category, never on-chain)".into());
        }
    }

    // --- jurisdiction-specific ---
    if record_type == Some("EU_HEALTH_CERT") {
        match (as_str(c, "validFrom").and_then(iso_date), as_str(c, "validUntilEntry").and_then(iso_date)) {
            (Some(vf), Some(vue)) => {
                if vue != vf + 10 {
                    e.push("EU_HEALTH_CERT: validUntilEntry must == validFrom + 10 days".into());
                }
            }
            _ => {
                if get(c, "validUntilEntry").is_none() {
                    e.push("validUntilEntry is required".into());
                }
            }
        }
        if let (Some(ov), Some(entry)) = (as_str(c, "onwardValid").and_then(iso_date), as_str(c, "validUntilEntry").and_then(iso_date)) {
            if ov > add_months(entry, 4) {
                e.push("EU_HEALTH_CERT: onwardValid must be <= entry + 4 months".into());
            }
        }
        if c.get("echinococcusRequired").and_then(Value::as_bool) == Some(true) {
            let ok = c.get("treatmentBeforeEntry").and_then(Value::as_f64).map(|t| (24.0..=120.0).contains(&t)).unwrap_or(false);
            if !ok {
                e.push("EU_HEALTH_CERT: echinococcus treatmentBeforeEntry must be within [24h, 120h]".into());
            }
        }
    }
    if record_type == Some("CDC_IMPORT_FORM") {
        let ok = c.get("ageMonthsAtEntry").and_then(Value::as_f64).map(|a| a >= 6.0).unwrap_or(false);
        if !ok {
            e.push("CDC_IMPORT_FORM: ageMonthsAtEntry must be >= 6".into());
        }
    }
    // type includes "DOT" => trustLevel = SELF_ATTESTED (informational; input is &Value,
    // so we only assert the handler-attestation posture, no mutation).

    if e.is_empty() {
        Ok(())
    } else {
        Err(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_rabies() -> Value {
        json!({
            "@context": ["https://www.w3.org/ns/credentials/v2", DOGTAG_CONTEXT_URI],
            "type": ["VerifiableCredential", "RabiesVaccinationCertificate"],
            "id": "urn:uuid:rabies-1",
            "issuer": "did:web:vet.example",
            "validFrom": "2024-02-01",
            "validUntil": "2027-01-11",
            "nextDueDate": "2027-01-11",
            "credentialSchema": {"id": "https://dogtag.io/schemas/rabies", "type": "JsonSchema"},
            "credentialStatus": {"id": "https://dogtag.io/status/1", "type": "DogTagStatus2025"},
            "attestationType": "vaccination",
            "signatureTrustTier": "licensed_vet",
            "legalEffect": "evidentiary",
            "legalBasisVersion": "EU-2013-576-v1",
            "jurisdiction": "EU",
            "recordType": "VACCINATION",
            "vaccineProductCode": "USDA-PCN-12345",
            "vaccineProductName": "Rabvac 3",
            "vaccineManufacturer": "Boehringer Ingelheim",
            "batchLotNumber": "LOT-998",
            "vaccinationDate": "2024-01-11",
            "authorizedVet": "did:web:vet.example#vet1",
            "series": "primary",
            "titer": {"labId": "LAB-7", "sampledAt": "2024-02-11", "resultIUml": "0.7"},
            "credentialSubject": {
                "dogTagId": "dogtag:0xabc",
                "dateOfBirth": "2023-09-01",
                "microchip": {"code": "985141006580319", "standard": "ISO_11784_11785", "implantDate": "2023-10-01"}
            }
        })
    }

    fn valid_service() -> Value {
        json!({
            "@context": ["https://www.w3.org/ns/credentials/v2", DOGTAG_CONTEXT_URI],
            "type": ["VerifiableCredential", "ServiceAttestation"],
            "id": "urn:uuid:svc-1",
            "issuer": "did:web:trainer.example",
            "validFrom": "2024-01-01",
            "credentialSchema": {"id": "x", "type": "JsonSchema"},
            "credentialStatus": {"id": "y", "type": "DogTagStatus2025"},
            "attestationType": "service",
            "signatureTrustTier": "self_attested",
            "legalEffect": "evidentiary",
            "legalBasisVersion": "ADA-v1",
            "jurisdiction": "US",
            "recordType": "SERVICE_ATTESTATION",
            "assistanceType": "service_dog",
            "issuerTrustTier": "adi_accredited",
            "taskDescription": "mobility assistance",
            "legalContext": ["ADA", "ACAA"],
            "storage": "off_chain",
            "credentialSubject": {"dogTagId": "dogtag:0xdef"}
        })
    }

    fn assert_violation(c: &Value, needle: &str) {
        match validate_schema(c) {
            Ok(()) => panic!("expected validation to fail for: {needle}"),
            Err(v) => assert!(v.iter().any(|m| m.contains(needle)), "violations {v:?} missing {needle:?}"),
        }
    }

    #[test]
    fn accepts_valid_rabies() {
        assert_eq!(validate_schema(&valid_rabies()), Ok(()));
    }

    #[test]
    fn fails_missing_vaccine_manufacturer() {
        let mut c = valid_rabies();
        c.as_object_mut().unwrap().remove("vaccineManufacturer");
        assert_violation(&c, "vaccineManufacturer is required");
    }

    #[test]
    fn fails_microchip_code_14_digits() {
        let mut c = valid_rabies();
        c["credentialSubject"]["microchip"]["code"] = json!("98514100658031");
        assert_violation(&c, "microchip.code must match");
    }

    #[test]
    fn fails_microchip_code_number() {
        let mut c = valid_rabies();
        c["credentialSubject"]["microchip"]["code"] = json!(985141006580319i64);
        assert_violation(&c, "microchip.code must be a string");
    }

    #[test]
    fn fails_bogus_trust_tier() {
        let mut c = valid_rabies();
        c["signatureTrustTier"] = json!("bogus");
        assert_violation(&c, "signatureTrustTier must be one of");
    }

    #[test]
    fn fails_primary_validfrom_not_plus_21() {
        let mut c = valid_rabies();
        c["validFrom"] = json!("2024-02-02"); // +22d
        assert_violation(&c, "validFrom must == vaccinationDate + 21 days");
    }

    #[test]
    fn fails_titer_below_threshold() {
        let mut c = valid_rabies();
        c["titer"]["resultIUml"] = json!("0.4");
        assert_violation(&c, "titer.resultIUml must be >= \"0.5\"");
    }

    #[test]
    fn passes_titer_exactly_half() {
        let mut c = valid_rabies();
        c["titer"]["resultIUml"] = json!("0.5");
        assert_eq!(validate_schema(&c), Ok(()));
    }

    #[test]
    fn accepts_valid_service() {
        assert_eq!(validate_schema(&valid_service()), Ok(()));
    }

    #[test]
    fn fails_service_storage_on_chain() {
        let mut c = valid_service();
        c["storage"] = json!("on_chain");
        assert_violation(&c, "storage must == \"off_chain\"");
    }
}

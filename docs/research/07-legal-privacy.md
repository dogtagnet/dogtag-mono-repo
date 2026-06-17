# 07 — Legal & Privacy Constraints on DogTag Data Structures and Architecture

**Status:** Research findings, 2026-06-17. Author: regulatory/legal research.
**Scope:** The legal/regulatory rules that constrain DogTag's *data structures* and *system design* — EU/US pet-movement law, US service-animal law, data privacy (GDPR/UK GDPR/CCPA-CPRA), electronic-credential legal validity (eIDAS 2.0 / ESIGN), and veterinary-records law. Primary-source URLs are inline.

> **Reading note / disclaimer.** This is engineering-facing legal research, not legal advice. Several of the sharpest items below rest on *draft* or *secondary* sources and are explicitly flagged. The two load-bearing conclusions — (a) **owner PII must never go on-chain**, and (b) a DogTag credential is **evidentiary, not self-authoritative** — are robustly supported by primary law and should be treated as hard design constraints.

---

## 0. Executive orientation

DogTag issues verifiable credentials for **(i) pet identity, (ii) rabies vaccination, (iii) EU/US travel health, and (iv) service-animal status**, anchored on an EVM chain with self-hosted vet/groomer backends and a consumer app. The legal landscape forces three structural decisions:

1. **A layered, accreditation-bearing issuer model** — pet-travel certificates are *not* single-issuer; they require an accredited vet's act *plus*, for export, a government counter-endorsement.
2. **A strict on-chain/off-chain split** — all natural-person PII off-chain; only non-personal commitments/status on-chain. This is dictated by the GDPR Art. 17 erasure right and Chapter V transfer rules colliding with blockchain immutability.
3. **An evidentiary (not authoritative) legal posture** — a DNS-bound, chain-anchored W3C VC is admissible evidence of integrity/timing, but it does *not* carry the legal presumption of a qualified EU seal/QEAA or a government credential, and a service-animal record is at most a *recorded self-attestation*.

---

## 1. EU Pet-Movement & Animal-Health Law

Two parallel regimes apply: the **non-commercial movement regime** (Reg (EU) 576/2013 + Implementing Reg 577/2013) and the **Animal Health Law (AHL) recodification** (Reg (EU) 2016/429 + delegated acts), which now governs identification/registration and "other-than-non-commercial" movements.

### 1.1 Regulation (EU) No 576/2013 — non-commercial movement
Primary text: [EUR-Lex CELEX:32013R0576](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32013R0576); retained-law mirror: [legislation.gov.uk/eur/2013/576](https://www.legislation.gov.uk/eur/2013/576/contents).

- **Identification (Art. 17(1) + [Annex II](https://www.legislation.gov.uk/eur/2013/576/annex/II)).** Dogs/cats/ferrets must be marked by an implanted **transponder** (or a clearly readable tattoo applied **before 3 July 2011**). The transponder must comply with **ISO 11784** (HDX or FDX-B) and be readable by an **ISO 11785**-compatible device. Where it does *not* conform, **the owner must supply the reading device** — so the schema cannot assume ISO conformance: capture `transponder_iso_compliant` and an optional reader reference.
- **Rabies-vaccination validity ([Annex III](https://www.legislation.gov.uk/eur/2013/576/annex/III)).** Valid only if: animal **≥12 weeks** at vaccination; vaccination date **does not precede** microchip application/reading date (microchip-first sequencing is legally dispositive — record *both* dates); validity starts **≥21 days** after completion of the primary protocol; a booster is treated as a *new primary* (re-triggering 21 days) unless given within the prior dose's validity.
- **Passport content (Art. 21(1)) & issuance (Art. 22).** The identification document must record: transponder/tattoo location, date, and **alphanumeric code**; species, breed, sex, colour, DOB, distinctive features; owner name/contact and signature; issuing **authorised veterinarian** name/contact/signature; rabies-vaccination details; **blood-sampling date** for the titer test; preventive-health-measure compliance. Per Art. 22 the issuing vet must **keep records for ≥3 years** — a statutory retention floor the schema must encode.

### 1.2 Rabies antibody titration test — Annex IV (entry from non-listed third countries)
Per [Annex IV](https://www.legislation.gov.uk/eur/2013/576/annex/IV) (referenced by Art. 10(1)(c)) and confirmed by the [EC entry-into-the-Union page](https://food.ec.europa.eu/animals/movement-pets/eu-legislation/entry-union_en):
- neutralising antibody **≥ 0.5 IU/ml** in serum;
- sample collected **≥ 30 days after vaccination**;
- movement **not less than 3 months (90 days)** after sampling (waived for EU return and movement between listed countries);
- test in a laboratory **approved under Article 3 of Decision 2000/258/EC** ([EUR-Lex 32000D0258](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32000D0258));
- result need not be renewed if the animal is revaccinated within validity.

Mandated titer fields: `lab_name`, `lab_approval_ref` (Decision 2000/258/EC), `sample_date`, `result_IU_ml` (≥0.5), and a derived `titer_still_valid` predicate dependent on an unbroken booster chain.

### 1.3 Commission Implementing Regulation (EU) No 577/2013
Primary text: [EUR-Lex CELEX:32013R0577](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:32013R0577).
- **Art. 3 + Annex III** — the **model pet passport** layout (our digital credential should map 1:1 to these sections).
- **Art. 4 + Annex IV** — the **model EU Animal Health Certificate** for non-commercial third-country entry (Part 1 model, Part 2 notes), **plus a separate owner written declaration** (Annex IV Part 3). Model AHC: [EC PDF](https://food.ec.europa.eu/system/files/2016-10/pm_non-com_model-animal-health-cert_en.pdf). The cert-plus-declaration structure means the schema needs a distinct `owner_declaration` artifact separate from the vet-issued health data.
- **Art. 2 + Annex II** — the **list of listed territories/third countries** (Part 1 / Part 2). Listing status drives whether the titer test is required, so the schema needs `country_of_origin` + `country_listing_status` evaluated against the live list.

### 1.4 The Animal Health Law recodification — who governs what
- **Reg (EU) 2016/429 (AHL)** ([EUR-Lex 32016R0429](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32016R0429)) — umbrella.
- **Delegated Reg (EU) 2019/2035** ([consolidated](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:02019R2035-20230406)) — **identification, registration, traceability** of kept dogs/cats/ferrets (Art. 70 identification methods) and establishment registration.
- **Delegated Reg (EU) 2020/688** ([EUR-Lex](https://eur-lex.europa.eu/eli/reg_del/2020/688/oj/eng)) — animal-health requirements for **intra-Union** movement; Annex VII restates the rabies conditions and adds origin from a registered/approved establishment, **no rabies in the 30 days pre-departure**, **clinical exam ≤48h before dispatch**, and **Echinococcus multilocularis** treatment for entry to FI/IE/MT/NO/Northern Ireland.

**Division of labour:** 576/2013 (+577/2013) = non-commercial movement + pet passport/AHC; 2016/429 + 2019/2035 = identification/registration/traceability + establishment approval; 2020/688 = intra-Union animal-health conditions. **The schema must record which legal basis a certificate was issued under**, because the field set and validity logic differ.

### 1.5 2024–2026 updates (flagged — verify against operative texts)
- **Delegated Reg (EU) 2026/131** ([EUR-Lex](https://eur-lex.europa.eu/eli/reg_del/2026/131/oj)) is cited by official sources as the current intra-EU pet-travel animal-health act — **confirm field-level effect.**
- Secondary sources report that from **22 April 2026** EU pet passports may be issued **only to owners with proof of EU residence**; non-EU residents (UK/US) must use a per-trip Animal Health Certificate → implies a mandatory `issuer_jurisdiction` / `owner_residence` eligibility field. ⚠️ *Confirm against the primary implementing act.*
- Echinococcus treatment window: **24–120 hours (1–5 days)** pre-travel.

### 1.6 EU mandated-field summary
| Element | Mandated fields | Cite |
|---|---|---|
| Microchip | alphanumeric code, ISO 11784/11785 conformity, location, application/reading date | Art. 17, 21(1)(a), Annex II |
| Animal | species, breed, sex, colour, DOB, distinctive features | Art. 21(1)(b) |
| Owner | name, contact, signature; + declaration for third-country entry | Art. 21(1)(c)(e); 577/2013 Annex IV Pt 3 |
| Rabies vaccine | product name/manufacturer, batch, vaccination date, validity start (≥21d), validity end | Annex III; 2020/688 Annex VII |
| Titer (non-listed origin) | approved lab + ref, sample date (≥30d post-vax), result ≥0.5 IU/ml | Annex IV; Dec. 2000/258/EC |
| Issuing vet | name, contact, signature, authorised/official status; retention **≥3 yrs** | Art. 21(1)(d), 22 |
| Origin/establishment | registered/approved establishment ID; clinical exam ≤48h; no-rabies-30d | 2019/2035 Art.70; 2020/688 |

---

## 2. US Dog Importation (CDC) & USDA APHIS Endorsement

### 2.1 CDC 2024 Dog Importation Rule (effective Aug 1, 2024)
Final rule "Control of Communicable Diseases; Foreign Quarantine: Importation of Dogs and Cats," published **May 13, 2024**, effective **Aug 1, 2024**, codified at **42 CFR §§71.50–71.51 (Part 71, Subpart F)** ([Federal Register doc 2024-09676](https://www.federalregister.gov/documents/2024/05/13/2024-09676/control-of-communicable-diseases-foreign-quarantine-importation-of-dogs-and-cats); operative section [42 CFR §71.51](https://www.law.cornell.edu/cfr/text/42/71.51); [CRS confirmation](https://www.congress.gov/crs-product/IN12485); [CDC hub](https://www.cdc.gov/importation/dogs/index.html)). *Note: the legacy "Part 98" naming circulates in secondary/draft sources; the operative regulation is **42 CFR §71.51**.* Basis: preventing reintroduction of the dog-maintained rabies virus variant (DMRVV, eliminated in the US in 2007).

**Codified universal requirements (§71.51):** ≥6 months old at arrival (§71.51(f)(1)); ISO-compatible microchip implanted **on or before** the date the current rabies vaccine was administered (§71.51(g)(1)–(2) — a validation constraint to enforce in schema); healthy on arrival (§71.51(i)(1)); complete CDC Dog Import Form submitted before arrival (§71.51(h)(1)).

**Universal requirements — every dog, every pathway** ([CDC](https://www.cdc.gov/importation/dogs/index.html)):
1. **Appears healthy** on arrival.
2. **≥ 6 months old** at entry.
3. **ISO-compatible (11784/11785) microchip**, scannable by a universal reader; **microchip number recorded first** on all forms.
4. A **CDC Dog Import Form** online submission receipt (obtained 2–10 days before arrival).

**CDC Dog Import Form fields** ([form guidance](https://www.cdc.gov/importation/php/dog-import-form/index.html)): importer name/contact; **microchip number**; dog name, breed, colour, sex, age/DOB; a **photograph** of the dog; **countries the dog lived in during the prior 6 months**; arrival date, US port of entry, airline/flight; attestation. The receipt is valid for multiple entries over a **6-month window** if the dog has only been in low-risk/dog-rabies-free countries.

**Pathway split — driven by where the dog has been (prior 6 mo) and where vaccinated** ([rabies-free/low-risk](https://www.cdc.gov/importation/dogs/rabies-free-low-risk-countries.html); [high-risk](https://www.cdc.gov/importation/dogs/high-risk-countries.html)):
- **Low-risk / dog-rabies-free only:** universal requirements only; **no CDC rabies certificate required**.
- **High-risk, vaccinated in US:** **"Certification of U.S.-Issued Rabies Vaccination"** form, **endorsed by USDA APHIS**; may arrive at any US port.
- **High-risk, vaccinated abroad:** **"Certification of Foreign Rabies Vaccination and Microchip"** form **AND** a **rabies serology titer (RNATT/FAVN ≥ 0.5 IU/mL)** from a **CDC-approved lab**; entry only at a port with a **CDC-registered Animal Care Facility (ACF)**, with reservation.
- **High-risk, foreign-vaccinated without valid titer:** must route through a CDC-registered ACF for exam/revaccination.

**Rabies-certification form data fields:** microchip (first); dog description; **vaccine product name, manufacturer, lot/serial, vaccination date, expiration/validity date**; photograph; **vaccinating vet name, license, signature**; for US-issued, the **USDA APHIS endorsement** (official + date); for foreign pathway, **serology lab, sample date, result ≥0.5 IU/mL, CDC-approved lab identity**.

**Receipt validity:** low-risk pathway → 6 months, **multiple entries** from the same country; high-risk pathway → **one-time use**. The CDC Dog Import Form is **importer self-attestation** (Section D, false-statement penalty language) — a different (lower) assurance level than the vet-issued + APHIS-endorsed rabies certs.

**Schema takeaway:** `microchip_iso` (mandatory primary key, recorded first); `dog_age_min_6mo` gate; microchip-implant-date **≤** vaccination-date validation rule (§71.51(g)(2)); **`countries_resided_last_6_months` (array) is legally load-bearing — it selects the entire requirement set**; `importation_purpose` enum (Personal/Commercial/Service/Government/Education-Exhibition-Research); `cdc_import_form_receipt_id` + `receipt_valid_until` + `receipt_single_use` flag; `dog_photo`; pathway-conditional rabies/serology blocks.

### 2.2 USDA APHIS accreditation & the National Accreditation Number (NAN)
**National Veterinary Accreditation Program (NVAP)**; governing regs **9 CFR Part 161** (and Part 160 definitions) ([9 CFR Part 161, LII](https://www.law.cornell.edu/cfr/text/9/part-161); [APHIS NVAP](https://www.aphis.usda.gov/nvap)). Two categories: **Category I** (companion animals/pets other than birds — typical for pet travel) and **Category II** (livestock, horses, food/fiber, birds, exotics). Accreditation is **state-specific and voluntary**; the vet must *also* hold a valid **state license** (accreditation ≠ licensure).

**National Accreditation Number (NAN):** a **six-digit, randomly generated** number (no correlation to the state license number), and it is **"required on all official documents that call for an accreditation number"** ([APHIS NAN](https://www.aphis.usda.gov/nvap/accred-number)).

**9 CFR §161.4 standards** ([LII](https://www.law.cornell.edu/cfr/text/9/161.4)) add schema-relevant temporal/integrity rules: (a) the vet must **personally inspect** the animal within **10 days** before issuing a certificate; (b) must not issue until **accurately and completely filled out**, and certificates are **valid for 30 days** after inspection; (c) when signing for another vet's work, must identify the performing vet (name, dates, location) and **retain copies** of lab results. *Flag: §161.4 sets the 30-day validity and a "retain copies" duty but does not state a fixed numeric retention term in the text reviewed — treat any specific federal-record retention-year figure as unverified; the underlying medical record's retention is governed by state board rules (commonly 3–5 yrs, §6).*

**Schema:** capture `usda_nan` (6-digit), `nvap_category` (I/II), `state_license_no`, `state_of_accreditation`, `accreditation_valid_until`; encode the 10-day-inspection and 30-day-certificate-validity rules as validation constraints.

### 2.3 VEHCS — electronic endorsement & the counter-signature requirement
**VEHCS = Veterinary Export Health Certification System** ([APHIS VEHCS](https://www.aphis.usda.gov/pet-travel/veterinary-export-health-certification-system); [APHIS pet-travel/export](https://www.aphis.usda.gov/pet-travel)). The legal chain for most export certs: (1) an **APHIS-accredited vet issues and signs** the health certificate; (2) it must be **endorsed (counter-signed) by a USDA APHIS Veterinary Services endorsing official** to be valid for export. The **EU requires** this APHIS endorsement; an endorsement-required certificate is **legally invalid without the APHIS counter-signature**. Endorsement adds: `aphis_endorsement { endorsing_official, endorsement_date, vehcs_certificate_id, endorsement_office }`.

### 2.4 The layered-issuer finding (core architectural consequence)
| Layer | Actor | Schema must capture | Authority |
|---|---|---|---|
| Clinical act | Vaccinating/examining vet (state-licensed) | vaccine product/lot/date/expiry, microchip, exam findings, license | State license |
| Federal certification | **USDA-accredited** vet | **NAN**, NVAP category, signature, cert type/number | 9 CFR 161 |
| Federal endorsement | **APHIS VS endorsing official** | endorsing official, date, **VEHCS cert ID**, office | APHIS VS |
| Importer attestation (CDC) | Importer | CDC Dog Import Form receipt ID, countries-last-6-mo, photo | 42 CFR 71.50 |

**US-export credentials are not single-issuer.** Accreditation (NAN) and APHIS endorsement (VEHCS) must be **mandatory, verifiable issuer attributes — not free text** — because a certificate lacking endorsement is legally invalid for destinations that require it. Microchip number is the universal primary identifier and (per CDC) the first datum recorded.

*Flagged:* exact printed field order on the CDC "Certification of Foreign Rabies Vaccination and Microchip" form was taken from CDC guidance rather than transcribed from the live PDF; per-country endorsement requirements vary (EU well-established).

---

## 3. Service Animals (US) — ADA & DOT ACAA

### 3.1 ADA definition (28 CFR §35.104 / §36.104)
A **service animal is a dog individually trained to do work or perform tasks for a person with a disability** ([ADA.gov Service Animals FAQ](https://www.ada.gov/resources/service-animals-faqs/)). **Emotional-support animals do NOT qualify** (comfort is not "work or tasks"). Miniature horses get a *separate conditional accommodation* (not "service animal" status) under [28 CFR §35.136](https://www.ecfr.gov/current/title-28/chapter-I/part-35/subpart-B/section-35.136). Only **two questions** are permitted (is it a service animal required for a disability; what task is it trained to perform); entities **may not require** documentation/certification. **No government or authoritative service-animal credential exists**, and private "registries" convey no ADA rights.

### 3.2 DOT Air Carrier Access Act — 14 CFR Part 382 (2020 final rule)
Final rule [85 FR 79742 (Dec 10 2020)](https://www.federalregister.gov/documents/2020/12/10/2020-26679/traveling-by-air-with-service-animals); definition at [14 CFR §382.3](https://www.law.cornell.edu/cfr/text/14/382.3): a service animal is **a dog individually trained** for a qualified individual with a disability — **excluding ESAs, non-dog species, and service-animals-in-training**. Airlines may require the **U.S. DOT Service Animal Air Transportation Form** (and, for flights ≥8h, the **Service Animal Relief Attestation**) ([transportation.gov](https://www.transportation.gov/individuals/aviation-consumer-protection/service-animals)).

### 3.3 The form is HANDLER self-attestation under false-statement penalty
The Air Transportation Form is signed by the **handler/passenger**, not a vet or any verifier. It warns that knowingly false statements are a **federal crime under 18 U.S.C. §1001** ([DOT form PDF](https://www.transportation.gov/sites/dot.gov/files/2021-01/U.S.%20DOT%20Service%20Animal%20Air%20Transportation%20Form.pdf)). Its legal weight derives from the **deterrent of criminal liability on the signer**, not from third-party verification.

### 3.4 CRITICAL trust analysis for an issuer system
1. **There is no authority-verified service-animal credential to issue.** ADA/DOT *forbid* requiring third-party certification and *deny effect* to private registries.
2. **The DOT form is self-attestation, not verification** — it shifts truthfulness risk onto the handler; it does not make the claim authority-attested.
3. **DogTag must not represent a service-animal record as "verified disability" or "verified service animal."** Doing so would misstate the law, imply a non-existent certification, and create misrepresentation/consumer-protection exposure. The most the system may legitimately do is **record and timestamp the existence of a signed self-attestation** — acting as an anchor/notary of a self-attestation, not an issuer of status.

### 3.5 Schema implications
- First-class, non-defaultable **`attestation_type`** enum (`self_attested` for service-animal vs `vet_certified` for rabies/health).
- **`signer_identity` = the handler** (subject is the attestor), explicitly not a licensed third party.
- **`penalty_acknowledgment`** + reference to the 18 U.S.C. §1001 warning + signature timestamp.
- **No `disability_verified` / `service_animal_certified` field**; ideally a schema-level `verified: false` for this class, with `legal_effect: evidentiary_self_attestation`.
- Capture the specific DOT form/version.
- A disability indicator is **GDPR Art. 9 special-category data** and likely CPRA "sensitive personal information" → never on-chain, heightened basis (see §4).

---

## 4. Data Privacy & PII (GDPR / UK GDPR / CCPA-CPRA)

### 4.1 GDPR (Reg (EU) 2016/679) — the core regime
- **Personal data & the pet-linkage problem (Art. 4(1)).** Personal data is any information relating to an identified or identifiable natural person, incl. an identification number ([consolidated text](https://eur-lex.europa.eu/eli/reg/2016/679/oj/eng)). **A microchip number, rabies-cert serial, or credential ID becomes the owner's personal data the moment it is reasonably linkable to the owner** (Recital 26 "means reasonably likely to be used"). You cannot assume a pet identifier is non-personal.
- **The vet/clinic as controller (Art. 4(7)).** The CNIL confirms that a participant who writes personal data to a blockchain in a professional capacity is a **controller** ([CNIL report summary, Inside Privacy](https://www.insideprivacy.com/financial-institutions/the-cnil-publishes-report-on-blockchain-and-the-gdpr/)). The issuing vet/clinic/groomer is controller for owner PII; the platform is likely **joint controller** (Art. 26) or processor (Art. 28) — must be pinned contractually. *Flagged unsettled:* controller/processor allocation on a permissionless chain is genuinely unresolved (EDPB urges resolving it at design time).
- **Lawful basis (Art. 6):** contract (6(1)(b)), legal obligation (6(1)(c), mandated vet records), consent (6(1)(a)) for optional processing. **A service-animal credential can reveal the owner's disability = Art. 9 special-category data** → higher Art. 9(2) bar.
- **Minimization & storage limitation (Art. 5(1)(c),(e))** apply even to on-chain identifiers; immutability makes storage-limitation especially hard on-chain.
- **Right to erasure (Art. 17) vs immutability — the central tension.** Art. 17 requires erasure "without undue delay" on listed grounds ([Art. 17, gdpr-info](https://gdpr-info.eu/art-17-gdpr/)). A blockchain cannot delete a committed entry. The EDPB is blunt that **technical impossibility is not a justification** for ignoring data-subject rights ([EDPB 02/2025 via activeMind](https://www.activemind.legal/guides/edpb-blockchain/)). **Conclusion: if personal data is on-chain you cannot guarantee Art. 17 compliance → personal data must not go on-chain.**

### 4.2 The blockchain-erasure resolution (EDPB 02/2025 + CNIL 2018)
**EDPB Guidelines 02/2025 on processing of personal data through blockchain technologies** ([landing page](https://www.edpb.europa.eu/our-work-tools/documents/public-consultations/2025/guidelines-022025-processing-personal-data_en); [PDF](https://www.edpb.europa.eu/system/files/2025-04/edpb_guidelines_202502_blockchain_en.pdf), adopted April 2025, consultation to 9 June 2025 — **flag: draft, clearest signal we have**) + CNIL 2018 converge:
- **Do not store personal data directly on-chain.** Keep it off-chain; put only "references or cryptographic evidence" (hashes/commitments) on-chain.
- **A hash/commitment of personal data can itself be personal data.** EDPB: "encrypted or hashed data remains personal and the GDPR continues to apply." The ICO concurs: **a salted hash is pseudonymisation, and pseudonymised data is still personal data** ([ICO pseudonymisation guidance](https://ico.org.uk/for-organisations/uk-gdpr-guidance-and-resources/data-sharing/anonymisation/pseudonymisation/)). **An unsalted hash of a low-entropy input (e.g., a 15-digit microchip number) is brute-forceable → effectively reversible → clearly personal data.** Hash a microchip number naively on-chain and you have put reversible personal data on an immutable ledger.
- **"Delete off-chain data + destroy keys/salt" as the accepted erasure mechanism (with caveat).** Keep PII + salt/key off-chain; on erasure delete the record *and* the salt/key so the on-chain commitment can no longer be reconstructed. CNIL calls this "close to the effect of" erasure but **explicitly not true erasure**; EDPB does not bless key-destruction as automatically satisfying Art. 17. *Flagged unsettled:* this is risk-mitigation, not a safe harbour — defensible only if the on-chain artefact is genuinely irreversible after key destruction and input entropy is sufficient. Document residual risk in a DPIA.
- **Mandatory DPIA**; EDPB prefers **permissioned** networks. A public EVM chain maximises cross-border exposure.

### 4.3 Cross-border transfer (Chapter V, Arts. 44–49)
Art. 44 bars transfer outside the EEA absent adequacy/safeguards/derogation ([Chapter V, gdpr-info](https://gdpr-info.eu/chapter-5/)). **A globally-replicated ledger is a structural Chapter V problem** — every node worldwide receives a full copy, and you cannot sign SCCs with anonymous node operators. **This is an independent reason, beyond erasure, that owner PII must never touch the ledger.** If nothing on-chain is personal data, there is no on-chain transfer.

### 4.4 UK GDPR / DPA 2018 (brief)
Substantively identical (personal data, Art. 17, Art. 5), regulated by the **ICO**, with the UK's own transfer regime (IDTA instead of EU SCCs). The ICO's 2025 anonymisation guidance is authoritative that salted hashes remain personal data. **Design for GDPR → satisfies UK GDPR; maintain a parallel UK transfer mechanism.**

### 4.5 CCPA / CPRA (Cal. Civ. Code §1798.100 et seq.)
- **"Personal information" (§1798.140)** is very broad: anything "reasonably capable of being associated with, or could reasonably be linked, directly or indirectly," to a consumer/household ([§1798.140 leginfo](https://leginfo.legislature.ca.gov/faces/codes_displaySection.xhtml?sectionNum=1798.140.&lawCode=CIV)) — sweeps in a credential/microchip ID tied to a California owner. **Deidentified/aggregated data is excluded.**
- **Right to delete (§1798.105):** delete within **45 days** of a verifiable request ([§1798.105 leginfo](https://leginfo.legislature.ca.gov/faces/codes_displayText.xhtml?division=3.&part=4.&lawCode=CIV&title=1.81.5)) — same immutability conflict, same off-chain resolution. The deidentification carve-out makes the salt+hash design *easier* to defend than under GDPR.
- **Applicability:** business if >$25M revenue, OR buys/sells/shares PI of ≥100,000 consumers/households, OR ≥50% revenue from selling/sharing PI. A consumer pet app may cross the 100,000 threshold early — **assume CCPA applies.**

### 4.6 What MUST NOT go on-chain (enumerated)
**Forbidden on-chain (never):**
- Any owner PII in cleartext (name, address, email, phone, account ID, payment data); owner-identifying free text or document scans.
- **Service-animal / disability indicators** (GDPR Art. 9; CPRA sensitive PI).
- *Unsalted* or low-entropy hashes of any owner-linkable identifier — **especially the microchip number** and rabies-cert serials.
- Plain hashes of PII generally ("hashed data remains personal").

**Permitted on-chain (only):**
- **Salted/keyed hashes or cryptographic commitments** (high-entropy input, off-chain destroyable salt/key) — risk-mitigation, not a guarantee.
- **Revocation/status registries** referencing credentials without owner PII.
- **DIDs / public keys** chosen so they are not themselves linkable to a natural person.
- Timestamps, schema/version IDs, issuer accreditation references (non-personal).

### 4.7 Accepted architecture
| Layer | Contents | Property |
|---|---|---|
| **On-chain (EVM)** | Salted commitments/hashes, revocation status, issuer DID/keys, schema version | Immutable, **no personal data** → no Art. 17 / Chapter V exposure |
| **Off-chain (vet/groomer backend = controller)** | All owner PII, raw microchip number, full payload, salts/keys, consent records, retention metadata | Encrypted, **deletable**, EEA/adequacy-located |
| **Erasure flow** | Delete off-chain record **+ destroy salt/key** → on-chain commitment becomes unlinkable | "Close to erasure" (CNIL); residual risk in DPIA |

*Adversarial caveats:* (a) EDPB 02/2025 is **draft**; (b) "delete key = erasure" is mitigation, not a blessed safe harbour; (c) whether a salted commitment with destroyed salt is truly "anonymous" vs "pseudonymous" is **legally unsettled** — design conservatively, assume in-scope.

---

## 5. Electronic-Credential Legal Validity (eIDAS 2.0 / ESIGN / W3C VC)

### 5.1 EU — eIDAS (Reg 910/2014) & eIDAS 2.0 (Reg 2024/1183)
Three-tier model: *simple* → *advanced (AdES)* → *qualified (QES)* electronic signatures, mirrored for **electronic seals** (the legal-person construct for an institutional issuer). Primary: [Reg 910/2014 consolidated](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX:02014R0910-20241018).
- **Art. 25(1)/46:** admissibility floor — not denied legal effect solely for being electronic. *Admissibility, not weight.*
- **Art. 25(2):** only a **QES** has the equivalent legal effect of a handwritten signature.
- **Art. 35(2):** a **qualified electronic seal** enjoys the **presumption of integrity and correctness of origin** — *reversing the burden of proof.* ([Art. 35](https://lexparency.org/eu/32014R0910/ART_35/)).

**Net:** a plain cryptographic VC proof is a *non-qualified* seal → admissible but **no presumption** (purely evidentiary; relying party must prove authenticity/integrity/attribution). Only the **qualified** tier (QES/QeSeal via a QTSP on an EU Trusted List) gets the burden-shifting presumption.

**eIDAS 2.0 (Reg (EU) 2024/1183, in force 20 May 2024)** introduces the **EU Digital Identity Wallet** (all 27 MS to offer by end-2026) and **Electronic Attestation of Attributes (EAA)** / **Qualified EAA (QEAA)** ([Art. 45](https://www.european-digital-identity-regulation.com/Article_45_(Regulation_EU_2024_1183).html)). **A W3C VC is functionally an EAA.** Art. 45b mirrors Art. 25: a non-qualified EAA is admissible/evidentiary; a **QEAA (or EAA from a public authentic source) has the same legal effect as a paper attestation.** To reach paper-equivalence a DogTag credential would have to be a **QEAA issued by a QTSP**.

### 5.2 US — ESIGN (15 U.S.C. §7001) & UETA
ESIGN §7001(a): a record/signature "may not be denied legal effect... solely because it is in electronic form" ([Cornell LII](https://www.law.cornell.edu/uscode/text/15/7001)); UETA is the state analogue. **Neither establishes any presumption of integrity or attribution** — authenticity/attribution must be proven under ordinary evidence law (UETA §9: attribution shown "in any manner," typically via audit trail). **No US QES/QeSeal equivalent.** US posture: valid + admissible but **purely evidentiary; proponent bears the burden.**

### 5.3 W3C VC + did:web + on-chain anchoring — standing
The [W3C VC Data Model 2.0](https://www.w3.org/TR/vc-data-model-2.0/) is structurally **evidentiary by design**: a VC is "tamper-evident" and cryptographically verifiable, but **"verifiability does not imply the truth of claims"** — trust/validation is *external* (verifier applies own business rules); DIDs incl. `did:web` are optional identifiers, not a source of legal authority.

**Conclusion:** a `did:web` (DNS-bound) + EVM-anchored credential is strong **technical evidence** — integrity, issuer-domain binding, tamper-evident timestamp/non-repudiation. But it does **not** confer the eIDAS Art. 35 presumption, QEAA paper-equivalence, or any US presumption. DNS binding proves "this domain published this," not "this issuer is an accredited authority." **Authority is extrinsic** — it comes from the issuer being a recognized/accredited body (USDA-accredited vet, EU authorised vet, QTSP), not from the chain. **Default posture: evidentiary, corroborating an underlying authority's act — not self-authoritative.**

### 5.4 Schema/architecture implications
1. **Strong, accreditation-bearing issuer identity** beyond `did:web` (USDA NAN, EU authorisation, QTSP/Trusted-List ID).
2. **Explicit signature/trust-tier field:** `none | AdES | QES | QeSeal | QEAA | public-authentic-source`.
3. **Qualified-issuance upgrade path** (QeSeal / QEAA into an EUDI Wallet) for cases needing paper-equivalent effect.
4. **Default = evidentiary**; product/legal copy must not over-claim "legally binding"/"government-grade."
5. **Anchor hashes, not credentials** — the chain is the integrity/timestamp witness (also satisfies the PII constraint in §4).
6. **`validFrom`/`validUntil`, issuer DID, accreditation refs as first-class fields** — exactly what a relying party must independently verify.

---

## 6. Veterinary Records Law

### 6.1 Ownership: record vs information (US)
Settled US split: **the practice/practice-owner owns the record; the client (animal owner) has a right to the *information* (copies/summaries).** AVMA *Principles of Veterinary Medical Ethics*: records are "the property of the practice and the practice owner," information is confidential and released only as law requires/allows or with owner consent ([AVMA Ethics](https://www.avma.org/resources-tools/avma-policies/principles-veterinary-medical-ethics-avma)). AVMA *Data Ownership and Stewardship* frames the practice as a **steward** of the underlying information ([AVMA Data Ownership](https://www.avma.org/resources-tools/avma-policies/principles-veterinary-data-ownership-and-stewardship)) — material when reusing vet-originated data to mint a credential.

### 6.2 Retention (representative US states; secondary source — verify against primary statutes)
Set by state practice acts, not federal law ([aggregator co.vet](https://co.vet/post/veterinary-medical-records-laws/)): California 3 yrs (B&P §4855); Texas 5 yrs; New York 3 yrs (8 NYCRR §70.2); Florida 3 yrs. **Safe design floor where silent: ≥5 yrs from last patient interaction** (AVMA recommendation). Controlled-substance/prescription records may require longer (DEA/state pharmacy).

### 6.3 Issuance authority & licensing (US + UK)
- Rabies/health records must be created and signed by a **licensed veterinarian**; a vet license is **jurisdiction-bound** (state board).
- For interstate/international (export) certs the signing vet must additionally be **USDA-APHIS accredited (NAN)**, and many certs require **APHIS endorsement** (VEHCS) — see §2.
- **UK (RCVS):** records "are the property of, and should be retained by, veterinary surgeons" ([RCVS clinical/client records guidance](https://www.rcvs.org.uk/setting-standards/advice-and-guidance/code-of-professional-conduct-for-veterinary-surgeons/supporting-guidance/clinical-and-client-records/)); POM-V/POM-VPS medicine records ≥5 yrs; RCVS *Certification* guidance ("Twelve Principles") governs a vet's integrity duties when certifying — relevant since a DogTag credential is a form of certification ([RCVS certification guidance](https://www.rcvs.org.uk/setting-standards/advice-and-guidance/code-of-professional-conduct-for-veterinary-surgeons/supporting-guidance/certification/)).

### 6.4 Schema implications
- **Record-custodian field** (practice/practice-owner = legal record owner) **distinct from** the **subject/client (animal owner)** with information-access rights — do not conflate.
- **Retention metadata:** `retention_period`, `retention_basis` (jurisdiction + statute), `retention_expiry` from `last_interaction_date` (default ≥5 yrs where silent).
- **Issuing-vet identity/authority mandatory:** `vet_license_number`, `license_jurisdiction`, `license_status`, and for export `usda_accreditation_number` (NAN) + endorsement reference.
- **Source-record reference** (pointer/hash to the underlying clinical record) rather than embedding the full confidential record.
- **Disclosure/consent flag** on contained information.

---

## 7. Synthesis — what the law REQUIRES or FORBIDS in our schema & architecture

### 7.1 Legally-mandated fields by credential type
- **Pet identity:** microchip alphanumeric code + **ISO 11784/11785 conformity flag** + implant location + application/reading date; species, breed, sex, colour, DOB, distinctive features. (EU Art. 17/21; CDC microchip-first.)
- **Rabies vaccination:** vaccine product/manufacturer, batch/lot, vaccination date, validity start (**≥21d / ≥12 wks at vax / after microchip date**), validity end; vaccinating vet name + **license**; for high-risk-US-import the **USDA endorsement**. (EU Annex III; 2020/688; CDC.)
- **EU/US travel health:** all of the above **plus** titer block where origin unlisted (approved lab + ref, sample date ≥30d post-vax, result ≥0.5 IU/mL, 3-month wait); **owner declaration** (577/2013); for US export the **layered issuer chain** (accredited vet NAN → APHIS VEHCS endorsement); CDC `countries_resided_last_6_months` (pathway driver) + import-form receipt; `country_listing_status`; `legal_basis_version`.
- **Service animal:** **NOT a verified credential** — `attestation_type = self_attested`, signer = handler, `penalty_acknowledgment` (18 U.S.C. §1001), specific DOT form/version, `verified: false`, `legal_effect: evidentiary_self_attestation`. **No `disability_verified` field.**

### 7.2 Hard privacy constraints — GDPR erasure vs immutability (resolution)
- **Owner PII off-chain only**, encrypted, deletable, EEA/adequacy-located.
- **On-chain: only** salted/keyed commitments (high-entropy input, off-chain salt), revocation status, non-personal issuer DIDs/keys, timestamps, schema/version, accreditation refs.
- **Never on-chain:** any owner PII, document scans, **disability/service-animal indicators**, and **unsalted/low-entropy hashes of the microchip number or cert serials** (reversible → personal data).
- **Erasure mechanism:** delete off-chain record **+ destroy salt/key** so the on-chain commitment is unlinkable. This resolves *both* Art. 17 erasure *and* Chapter V transfer — but is **risk-mitigation, not a regulator-blessed safe harbour**; document in a **mandatory DPIA**; prefer a **permissioned** EVM network.

### 7.3 Legal-validity posture
**Evidentiary by default, not authoritative.** A DNS-bound, chain-anchored W3C VC is admissible and proves integrity/timing/origin-claim, but carries **no legal presumption** under eIDAS (only a QES/QeSeal/QEAA via a QTSP does) and none under US ESIGN/UETA. Authority is **extrinsic** — it flows from the accredited issuer (vet/APHIS/competent authority), not the blockchain. Provide a **qualified-issuance upgrade path** for cases needing paper-equivalent effect; never market the baseline as "legally binding/government-grade."

### 7.4 Concrete schema/architecture requirements (checklist)
1. **PII strictly off-chain**; on-chain = salted commitments + non-personal metadata only.
2. **Salt/key management with an audited destruction path** (erasure = delete data + key).
3. **Per-purpose lawful-basis + consent records** (granular, withdrawable, timestamped), off-chain.
4. **Retention fields** per credential: custodian, basis (jurisdiction + statute), retention clock (default ≥5 yrs / EU ≥3 yrs).
5. **`legal_basis_version` / jurisdiction-versioning field** on every credential (which regulation + version it was issued under) — EU vs US, 576/2013 vs AHL, CDC pathway, form version.
6. **Issuer accreditation fields MANDATORY (not free text):** `usda_nan`, `nvap_category`, `vet_license_number` + `license_jurisdiction`, `accreditation_valid_until`, and for export the **APHIS endorsement block** (`vehcs_certificate_id`, endorsing official, date).
7. **Layered/multi-issuer model** for export certs (accredited-vet issuance + APHIS counter-seal as a second attestation).
8. **`attestation_type` + `signature_trust_tier` + `legal_effect`** first-class fields; service-animal records flagged `self_attested` / `verified:false`.
9. **Record-custodian distinct from pet-owner**; `source_record_reference` (hash/pointer) rather than embedding confidential records.
10. **CCPA deletion endpoint (45-day SLA)** wired to the same off-chain delete + key-destroy mechanism.
11. **Treat service-animal/disability data as special-category** — heightened lawful basis, opt-in, never on-chain.
12. **Mandatory DPIA**, refreshed on any change to on-chain fields or chain topology; prefer permissioned network.

---

## 8. Open items to verify before production reliance
- **Delegated Reg (EU) 2026/131** field-level effect and the **22 April 2026 EU-residence** passport-eligibility rule (currently part-secondary).
- Exact **CDC foreign-vaccination form** field layout (transcribe from live PDF) and per-country APHIS endorsement requirements.
- Per-state **veterinary retention statutes** (re-verify against primary board rules; aggregator used).
- **EDPB Guidelines 02/2025** are *draft* — track the adopted final version; the "key-destruction = erasure" theory's defensibility remains legally unsettled.
- eIDAS 2.0 **QEAA conformance profiles / EUDI ARF** were still finalizing through 2025–2026.

# DogTag — Competitive Apps & Standards: Critical Analysis and Finalized Field Set

> **Purpose.** Critically analyze real-world pet-credentialing, pet-record, microchip, pet-travel, service-dog, and verifiable-credential health-pass products/standards, and use the findings to finalize the DogTag on-chain data schema.
> **Scope.** Six domains: (1) pet health-record apps, (2) microchip registries + ISO 11784/11785, (3) pet-travel/passport platforms, (4) service-dog registries + DOT form, (5) blockchain / verifiable-credential health precedents, (6) W3C VC 2.0 + FHIR Immunization modeling.
> **Companion docs.** This complements `01-data-standards.md` (regulatory/legal field mandates) and `02-attestation.md`/`03-chain-contracts.md` (crypto + on-chain mechanics). Where this doc and `01` overlap, `01` is the legal authority and this doc is the product/critical-analysis lens.
> **Date of research.** 2026-06-17.
> **Baseline under critique — the xlsx-derived schema** (`references/data_references/Tables for Onchain Deployment.xlsx`):
> - **DogProfile**: `Dog Tag ID (SBT)`, `Dog Name`, `Breed`, `Service Dog (Y/N)`, `Date of Birth`, `Weight`, `Microchip ID`, `Whitelist`.
> - **Vet/Issuer**: `Dog Tag Issuer ID`, `USDA Accreditation number`, `Name`, `Veterinary Hospital`, `address`, `telephone number`, `local license number`, `Whitelist`.
> - **Vaccine credential (W3C VC)**: `Dog Tag ID`, `Issuer`, `@context`, `type`, `validFrom`, `validUntil`, `Vaccination type`, `Vaccine product name`, `Batch/Lot number`, `Lot expiration date`, `Name`, `Species`, `Sex`, `Age (at time of vax)`, `Breed`, `Weight (at time of vaccine)`, `Microchip ID`.
> - Sample row values: `Breed = "Golden Doodle"`, `Weight = 50` (no unit), `Age = "2 years, 5 months, 10 days."` (free text), `Sex = "Male Neutered"`, `Microchip ID` stored as a **float** (`9.85141006580311E14` → precision loss).

---

## 1. Pet Health-Record / Vaccination Apps

These products define what pet owners and clinics actually capture day to day. Two archetypes dominate: **clinic-PIMS-synced apps** (data flows from the vet's practice-management system; the owner is read-only) and **owner-entry vaults** (the owner types everything; rich but unverified).

### Per-product findings

**PetDesk** — a clinic-tethered communication app. Owners "view records, labs, and vaccines," reminders, and prescription history, but **records are created/updated only in the provider's practice-management software**; an owner cannot edit, and changes sync into the app **24–48 hours** after the clinic updates. Matching is by **shared email** between clinic and app. ([petdesk.com](https://petdesk.com/download-app-for-pet-health), [petdesk.zendesk.com](https://petdesk.zendesk.com/hc/en-us/articles/360052833813-Accessing-or-Editing-Pet-Reminder-Prescription-Records))
- *Strength:* data provenance is the clinic (authoritative). *Gap:* total data lock-in to one clinic's PIMS; if the record isn't in that clinic, it doesn't exist in the app; no portability, no cross-clinic merge, no verification primitive a third party could trust.

**11pets** — the richest **owner-entry** model. Multi-pet, multi-species household organizer. Per pet it tracks: weight + vital signs + arbitrary **measurements with normal-range warnings and trend charts**; **vaccinations and deworming with automatic due-date schedules and reminders**; **medications (drug, dosage, frequency, per-administration notifications)**; flea/tick treatments; medical incidents, allergies, conditions, surgeries; and uploaded documents (x-rays, bloodwork, tests). Data is **shareable with a vet or caregiver**. ([11pets.com/feature](https://www.11pets.com/en/feature), [11pets.com](https://www.11pets.com/en/pet-care-app), [Google Play](https://play.google.com/store/apps/details?id=com.m11pets.elevenpets))
- *Strength:* comprehensive field model, true multi-pet/species, expiry reminders, document attachments. *Gap:* every datum is self-entered → **no verification**; useless as a credential anyone else can trust.

**Pawprint** (Mars Petcare lineage), **VitusVet**, **PetNoter**, **PetPassport** — self-uploaded record vaults; some fetch records from clinics. They share 11pets' verification gap (cited in travel synthesis below). ([padspass comparison context](https://www.padspass.com/digitalpetpassport))

**Vetstoria** — a **booking/scheduling** engine that integrates into clinic websites/PIMS; it is an appointment layer, not a health-record store. Relevant only as the calendar/appointment analogue (see `05-calendar-appointments.md`).

**Petbooqz** — a veterinary practice-management system (PIMS) + an owner app; same clinic-as-source-of-truth model as PetDesk.

**AKC Reunite app** — recovery-oriented (microchip + contacts), covered in §2.

**EU digital pet-passport initiatives** — see §3: as of 2026 there is **no official digital EU pet passport**; the passport remains a paper booklet even after the 2026 reissue. Third-party apps (PadsPass, Boop) are filling the gap with vet-verified records but have **no government recognition**.

### What fields they actually store (cross-product)

| Field group | Fields commonly stored | DogTag xlsx has it? |
|---|---|---|
| **Pet identity** | name, **species**, breed, **sex (+ neuter status)**, DOB, **color/markings**, **photo(s)** | partial — no species(top-level), no color, no photo |
| **Pet biometrics** | **weight over time (a series, with units + date)**, body condition, vitals | xlsx stores a single scalar `Weight`, no unit, no date |
| **Microchip** | number, **brand**, **implant date**, **body location** | only the number (and as a float) |
| **Vaccination** | vaccine name, **manufacturer**, lot/batch, **date given**, **due/next-due date**, expiry, administering vet, **document upload** | name/lot/lot-expiry/validFrom/validUntil present; no manufacturer field of its own; no due-date; no document hash |
| **Owner** | name, address, phone, email, **emergency/alternate contact** | **absent entirely — no owner entity** |
| **Vet/clinic** | clinic name, address, phone, vet name, license | present in Vet table |
| **Multi-pet / species** | first-class (households, multiple species) | single-species (dog) assumed |
| **Vaccine coding** | **free text** (no app uses CVX or a product registry) | free text |

### Cross-cutting gaps & pain points (synthesis)

1. **No verification / no portability.** Owner-entry apps are unverified; clinic apps are verified but locked to one PIMS. **Neither produces a credential a third party (airline, border, new vet) can cryptographically trust.** This is the exact whitespace DogTag targets.
2. **No standardized vaccine coding.** Every app uses free-text vaccine names ("Nobivac", "Rabies") — no CVX, no product registry code → can't deduplicate, can't validate, can't compare across jurisdictions.
3. **Weight modeled as a unitless scalar**, not a dated series with units — a recurring data-quality bug that the DogTag xlsx reproduces.
4. **Expiry/due-date handling is inconsistent** — 11pets does it well (auto-schedule + reminders); the DogTag xlsx vaccine record has `validUntil` but **no "next due" date**, which is what the CDC/USDA forms actually require (see §3).
5. **Owner is implicit or buried.** Apps tie a pet to one account; few model the **owner as a separable, transferable entity** — yet ownership transfer is the central lifecycle event for a microchip/identity system (§2).

---

## 2. Microchip Registries & ISO 11784/11785

### ISO 11784/11785 — the 15-digit code

ISO 11784 defines the **data structure**; ISO 11785 the **air-interface protocol**. A compliant chip is a **64-bit code rendered as 15 decimal digits**:
- **First 3 digits = ISO-3166 country code OR an ICAR-issued manufacturer code.** Prefixes **900–998 are manufacturer codes, not countries**; **900 is a *shared* code used by 100+ small manufacturers** (each gets a 1-million-ID block); **999 is reserved for test transponders and is explicitly NOT guaranteed unique**. ([Wikipedia ISO 11784/11785](https://en.wikipedia.org/wiki/ISO_11784_and_ISO_11785), [maxmicrochip.com](http://maxmicrochip.com/ISO_types.htm), [service-icar.com](https://www.service-icar.com/tables/Tabella3.php))
- **Remaining 12 digits = the unique animal ID** (38-bit national/animal ID field). ([Wikipedia](https://en.wikipedia.org/wiki/ISO_11784_and_ISO_11785))
- **FDX-B vs HDX**, both at 134.2 kHz; FDX-B (128-bit, ASK) is the common pet format. ([Wikipedia](https://en.wikipedia.org/wiki/ISO_11784_and_ISO_11785))
- **Non-ISO legacy chips** (AVID 9-digit 125 kHz encrypted, HomeAgain 125 kHz, 128 kHz) still circulate in the US and may not be readable by a "forward-reading" ISO scanner. ([peeva.co timeline](https://peeva.co/blog/strategic-incompatibility-a-timeline/), [AVMA](https://www.avma.org/resources-tools/pet-owners/petcare/microchips-reunite-pets-families/microchipping-faq))

**Critical: uniqueness is an honour system, enforced at neither layer.** ISO does not actually appoint an allocation authority; ICAR *coordinates voluntarily*. The shared 900 code, 999 test collisions, and **legal recycling of numbers every 33 years** mean global uniqueness is aspirational, not guaranteed. ([rfidnews.com](https://www.rfidnews.com/ISOstandard/ISOstandard.html), [Animal ID Corp](https://intercom.help/animal-id-corporation/en/articles/10498528-microchip-numbering-rules-standards-and-counterfeits))

### Registry field models & the fragmentation problem

| Registry | Pet fields | Owner fields | Alt contacts | Model |
|---|---|---|---|---|
| **AKC Reunite** | pet record + collar tag w/ ID | address, phone(s) | yes | $19.50 **lifetime**, no update fee, all brands, 24/7 recovery ([akcreunite.org FAQ](https://www.akcreunite.org/microchippingmypetfaq/), [akc.org](https://www.akc.org/products-services/akc-reunite/)) |
| **HomeAgain** | description | name, phone, address | secondary | $19.99/yr services; chip lifetime regardless; free updates; **$3.5M class-action settlement** over implying paid membership was required ([classaction.org](https://www.classaction.org/news/home-again-deceives-pet-owners-into-thinking-paid-membership-is-necessary-to-access-microchip-database-class-action-alleges)) |
| **24PetWatch / Found Animals** | pet record | name, contact | yes | free base registration, all brands ([foundanimals.org](https://www.foundanimals.org/microchip-register-faqs/)) |
| **Petlog (UK)** | pet record | keeper address + phone | — | Defra-mandatory; one of *several* competing UK databases ([petlog.org.uk](https://www.petlog.org.uk/news-and-stories/helpful-articles/compulsory-cat-microchipping/)) |
| **AAHA Universal Lookup** | — | **none returned** | — | free **locator only** — tells you *which registry* holds the chip, not owner data ([aaha.org](https://www.aaha.org/for-veterinary-professionals/microchip-search)) |

Every registry models the same graph: **Pet ↔ Microchip(number, brand) ↔ Owner(name, phones, email, address) ↔ Alt contacts.** The **AAHA tool returns no owner data** — only which registry to call — and **only searches registries that opt in**. **PetMaxx** is the separate international locator. ([foundanimals.org AAHA tool](https://www.foundanimals.org/aaha-universal-pet-microchip-lookup-tool/))

**The pain points (design-critical):**
1. **No central US registry** — each manufacturer keeps its own DB ([AVMA](https://www.avma.org/resources-tools/pet-owners/petcare/microchips-reunite-pets-families/microchipping-faq)).
2. **Stale owner contact data is the #1 reunification failure** — a chip pointing to a dead phone is "as good as unregistered." Current owner contact is the **scarce, decaying asset**, not the chip number.
3. **~60% registration rate** — many chips never registered at all.
4. **Same chip in multiple registries** with no reconciliation; registry shutdowns (Save This Life) orphan records.

**Lesson for DogTag:** treat the **microchip number as a non-authoritative lookup key** (it can collide, be recycled, or be unreadable), and treat **current verified owner binding + transfer history** as the thing of real value. A blockchain's genuine advantage here is a single, tamper-evident, transferable owner-binding record — not the chip number itself.

---

## 3. Pet-Travel / Passport Platforms & Government Import Systems

Four non-interoperating layers joined only by the microchip number as a de-facto key no system shares.

### EU Pet Passport (Annex III) — note the 2026 reissue

The model you referenced (Reg (EU) 577/2013, Annex III) was **repealed and replaced by Commission Implementing Reg (EU) 2026/705 (22 Apr 2026)**; passports issued before 1 Jan 2028 stay valid for life. The replacement is **still a paper booklet** (no digital format, no QR). ([EUR-Lex 2026/705](https://eur-lex.europa.eu/eli/reg_impl/2026/705/oj), [food.ec.europa.eu](https://food.ec.europa.eu/animals/live-animal-movements/dogs-cats-and-ferrets/travelling-pet-within-eu_en))

Section structure (Annex III; field detail from DAERA PT1 guidance): **I Ownership** (owner name/address/signature, re-sign on transfer) · **II Description** (name, species, breed, sex, DOB, coat colour/markings, optional photo) · **III Marking** (microchip code, application/reading date, transponder location) · **IV Issuing** · **V Rabies vaccination** (manufacturer, name, batch/lot, date, valid-from, valid-until) · **VI Titration** · **VII Echinococcus (date + time)** · **VIII–XII** other. ([EUR-Lex 577/2013](https://eur-lex.europa.eu/legal-content/EN/TXT/HTML/?uri=CELEX%3A32013R0577), [DAERA PT1](https://www.daera-ni.gov.uk/sites/default/files/publications/daera/PT1%20NI%20-%20How%20to%20Complete%20Pet%20Passports%20-%20v6.0%20(Dec%2022)_0.PDF))

### USDA APHIS VEHCS

A **USDA-Accredited Veterinarian** (not the owner) logs in via login.gov; VEHCS **blocks issuance if license/accreditation is expired**. Captures: routing (destination country, tracking #), origin (vet license + accreditation no., inspection date), **Consignor (owner/exporter) + Consignee** records, shipping, a **per-animal commodities table** (species, breed, sex, DOB/age, color/markings, **microchip number + implant date**), **rabies fields** (product, manufacturer, lot, expiry, date administered, **date next due**) — and **will not accept a vaccination dated before the microchip implant date**. Output banner color (GREEN fully digital → ORANGE/RED ink-and-embossed-paper-by-mail). ([aphis.usda.gov/pet-travel/vehcs](https://www.aphis.usda.gov/pet-travel/vehcs), [VEHCS create-certificate PDF](https://vehcs-training.aphis.usda.gov/VEHCSHelp/VEHCS_Create_Certificate.pdf))

### CDC Dog Import Form / DogBot (rule effective Aug 1, 2024)

Baseline for every dog: healthy, **≥6 months old**, **ISO-readable microchip implanted before the rabies vax** (or vax invalid), and a **CDC Dog Import Form receipt**. Form fields (OMB 0920-1383): importer (name, DOB, ID), dog (name, age, sex, breed, color/markings, **microchip number for high-risk**), **photo showing face+body (high-risk, ≤90 days, ≤15 days for under-1-year)**, travel/location history (all countries in prior 6 months), arrival, port. **DogBot** is an informational navigator only. Receipt validity: low-risk 6 months/multi-entry (same country); high-risk single entry on the listed arrival date. ([cdc.gov dog import form instructions](https://www.cdc.gov/importation/dogs/dog-import-form-instructions.html), [cdc.gov DogBot](https://www.cdc.gov/importation/dogs/dog-importation-navigator.html))

The CDC **US-issued Rabies Vaccination form** is the most authoritative US field list: dog **name, microchip number, microchip implant date, breed, DOB, sex, color**; vaccine **product name, manufacturer, lot number, product expiration, date administered, date next due**; vet **name, USDA Category I/II accreditation, state license, inspection date, certification checkboxes**. ([cdc.gov instructions](https://www.cdc.gov/importation/hcp/dog-importation/instructions-us-issued-rabies-vaccination-form.html))

### Airline / aggregator tools

- **United** "Saved Pets" profile: name, species, breed, **weight**, age, kennel L×W×H, microchip (intl). Most carriers **re-key** name+breed+weight and don't persist microchip/sex/DOB (AA by phone). **GoodDog is a breeder marketplace, not travel.** **BringFido** is lodging; "My Pets" stores **no health/microchip data**. ([united.com saved pets](https://www.united.com/en/us/account/profile/savedpets/petinfo/1/), [bringfido.com](https://www.bringfido.com/privacy/))
- New verification-adding apps: **PadsPass** (vet "CheckPoint" verification, but **no government interop**) and **Boop** (vet e-signed vaccination records). ([padspass.com](https://www.padspass.com/digitalpetpassport))

### Critical synthesis — the travel thesis

1. **Everyone re-captures the same data** (microchip, rabies date/lot/manufacturer, breed/sex/age, owner) into EU passport, VEHCS, CDC form, each airline, each app — none propagate.
2. **No interoperability with systems of record** — the only machine link is CDC ↔ USDA VEHCS.
3. **No authenticity verification** — certs/receipts are PDFs/paper checked by human eye.
4. **Fraud is quantified:** EU 2023 enforcement report — €4.6bn trade, **467 fraud notifications/yr (17% passport/ID forgery, 9% forged titration reports, 5% forged health certs)**, microchips found taped to fur; the Commission explicitly calls for "technological means… for cross-border access to reliable traceability information on individual animals." ([EU illegal-trade report PDF](https://food.ec.europa.eu/system/files/2023-12/agri-fraud_report_Illegal-trade_cats-dogs.pdf), [FOUR PAWS](https://www.four-paws.org/our-stories/press-releases/november-2024/the-illegal-puppy-trade-in-europe-a-billion-euro-industry-that-harms-dogs-and-humans-alike))
5. **A "date next due" is a required field** in both the CDC and VEHCS rabies records — the DogTag xlsx omits it.
6. **Microchip-before-vaccination ordering is mandatory** and machine-enforced by VEHCS — the schema must carry `microchipImplantDate` to validate this constraint.

---

## 4. Service-Dog "Registration" & the DOT Form — the Trust Problem

### Online "registries" are legally meaningless

A cottage industry (US Dog Registry, US Service Animals, National Service Animal Registry, ESA-letter mills) sells ID cards/certificates/vests. **DOJ is explicit** (ADA FAQ): covered entities **may not require** registration/certification; "**These documents do not convey any rights under the ADA and the Department of Justice does not recognize them as proof that the dog is a service animal.**" Requiring registration is itself an ADA violation. ([ada.gov service-animals FAQ](https://www.ada.gov/resources/service-animals-faqs/), [ada.gov 2010 requirements](https://www.ada.gov/resources/service-animals-2010-requirements/))

**The trust problem:** these are self-asserted credentials with **zero verification gate** — payment is the only requirement. A "verification portal" is circular (it only confirms someone paid the same vendor). **Putting a fake-registry attestation on a blockchain just makes a worthless claim immutable** — immutability does not create truth. The trust value of a credential is determined almost entirely by **the issuer's verification process.**

### The DOT Service Animal Air Transportation Form (14 CFR § 382.75, eff. Jan 11 2021)

Airlines may require this form (OMB 2105-0576). Sections: **A** required-because-of-disability; **B** identification & health — handler/user name+phone, animal name, **description (weight, breed/color)**, **veterinarian name+phone, date of last rabies vaccination**, attestation animal is vaccinated and disease-free; **C** task training (trainer name+phone, may be the handler — self-training is lawful); **D** behavior; **E** signature + 18 U.S.C. § 1001 false-statement warning. For flights ≥8 hours, an additional **Relief Attestation Form**. ([law.cornell.edu 14 CFR 382.75](https://www.law.cornell.edu/cfr/text/14/382.75), [transportation.gov final rule](https://www.transportation.gov/briefing-room/us-department-transportation-announces-final-rule-traveling-air-service-animals))

**Critical: the DOT form is a HANDLER SELF-ATTESTATION, not a third-party credential.** No vet, doctor, or trainer signs. Its trust mechanism is **legal accountability** (federal-crime false-statement liability), not verification.

### Three candidate issuers, ranked by trust

| Issuer | Trust | Why |
|---|---|---|
| Private "registry" | ❌ negative | No verification; DOJ says worthless/scam |
| Handler self-attestation (DOT-form model) | ⚠️ conditional | Credible via legal liability (18 U.S.C. § 1001), not verification |
| **Accredited training org (ADI)** | ✅ strongest | Org actually trained/assessed the team and is itself independently audited & re-accredited every 5 yr ([assistancedogsinternational.org](https://assistancedogsinternational.org/standards/what-is-accreditation/)) |

ADI cannot be made *mandatory* (the ADA permits self-training), so it's a strong positive signal, not a gate. **Legal context (ADA public access / ACAA air / FHA housing) and the service-dog-vs-ESA distinction must be first-class in the schema** — an ESA is a housing-only construct and is a *pet* for air travel/ADA. Conflating them is the error the scam registries exploit.

**Lesson for DogTag:** a boolean `Service Dog (Y/N)` is dangerously naive. The credential must record **who attested, by what process, under what legal context** — i.e. a *service-dog attestation credential* with an issuer trust tier (ADI-accredited org vs. handler self-attestation), explicit task description, and the legal context, and must **not** be confused with the buy-a-certificate model.

---

## 5. Blockchain / Verifiable-Credential Health Precedents

### EU Digital COVID Certificate (EU DCC) — the planet-scale gold standard

Vaccination element `v` — 10 fields, all using **controlled value sets** (canonical codes, not free text): `tg` target disease (SNOMED), `vp` vaccine type (SNOMED), `mp` medicinal product (EU register), `ma` manufacturer/MAH (EMA SPOR Org ID), `dn`/`sd` dose number/total, `dt` date, `co` country (ISO-3166), `is` issuer, `ci` unique cert ID. Encoding: **JSON → CBOR (CWT) → COSE_Sign1 (ES256) → zlib → Base45 → `HC1:` QR**. Trust: two-layer national PKI (CSCA→DSC) distributed via a daily-refreshed **DCC Gateway** trust list enabling **offline verification with no central holder database**. ([eu-dcc-schema](https://github.com/ehn-dcc-development/eu-dcc-schema), [eu-dcc-valuesets](https://github.com/ehn-dcc-development/eu-dcc-valuesets), [HCERT spec](https://github.com/ehn-dcc-development/eu-dcc-hcert-spec/blob/main/hcert_spec.md), [Decision 2021/1073](https://eur-lex.europa.eu/legal-content/EN/TXT/?uri=CELEX:32021D1073))
- *Lessons:* privacy-by-design (no central DB) made **revocation hard** (bolted on later); biggest real-world breach was a **process attack (cloned issuance pages), not crypto**; controlled value sets are what made the same fact hash and verify identically across jurisdictions.

### SMART Health Cards (VCI)

W3C VC wrapper → **FHIR R4 Bundle** (Patient + Immunization resources, `vaccineCode` = **CVX** `http://hl7.org/fhir/sid/cvx`) → **compact JWS (ES256)**, payload raw-DEFLATE, QR `shc:/` numeric mode. Trust directory: issuers publish JWKS at `<iss>/.well-known/jwks.json`; the **VCI Directory** lists vetted issuers + daily offline snapshot. **They deliberately sign the compact serialized bytes and reconstruct JSON-LD only for display — avoiding RDF canonicalization entirely.** ([spec.smarthealth.cards](https://spec.smarthealth.cards/), [smart-on-fhir/health-cards](https://github.com/smart-on-fhir/health-cards/blob/main/docs/index.md))

### Pet-specific blockchain — honest verdict: ~vaporware

No credible adoption. **Pawtocol (UPI)** token ~−99.99% / ~$15K cap; **PetChain** an unfunded Cardano proposal; "Pet ID", "dog-registry-blockchain-app" hackathon/bootcamp demos; no real pet SBT project. The US incumbent (AKC Reunite) is a plain database. **Livestock** traceability is more real but has a poor survival record (BeefChain defunct; AgriDigital abandoned blockchain); the systems that actually run nationally (Australia NLIS, EU TRACES) are deliberately **not** blockchain. ([CoinMarketCap Pawtocol](https://coinmarketcap.com/currencies/pawtocol/), [BeInCrypto](https://beincrypto.com/how-agriculture-focused-blockchain-products-are-actually-doing-today/))

### Canonicalization lessons (load-bearing for on-chain anchoring)

Signatures/anchors hash **bytes**; `{"a":1,"b":2}` and `{"b":2,"a":1}` hash differently though identical. Options: **JCS (RFC 8785)** (sort keys, ECMAScript numbers, UTF-8); **RDF Dataset Canonicalization (RDFC-1.0)** used by VC Data Integrity but carrying a **graph-isomorphism / dataset-poisoning DoS** surface (rdf-canon §7.1); **JWS/COSE compact** — the transmitted bytes *are* canonical (SHC + EU DCC). CBOR has **three incompatible "canonical" orderings** — pin one. ([RFC 8785](https://www.rfc-editor.org/rfc/rfc8785), [W3C rdf-canon](https://www.w3.org/TR/rdf-canon/), [RFC 8949](https://www.rfc-editor.org/rfc/rfc8949))

**Concrete rules for DogTag:**
1. Define exactly **one** canonical byte serialization, hash that, anchor the hash — never re-serialize on the verify side.
2. **Prefer signing the compact bytes directly** (JOSE/COSE), as SHC and EU DCC do — eliminates a whole class of canonicalization-mismatch and DoS bugs. Avoid RDF/JSON-LD canonicalization for on-chain workflows.
3. **Use controlled value sets** (à la EU DCC's SNOMED/EMA codes) for disease/vaccine/manufacturer so the same fact always hashes the same.
4. **Anchor only a 32-byte hash (or Merkle root) on-chain; store the full credential off-chain.** "Store all pet records on the blockchain" is an anti-pattern (cost, scale, GDPR-immutability conflict).
5. Use a **published issuer key directory** (VCI-style) for offline verification; bolt in **revocation from day one** (EU DCC's hardest retrofit).

---

## 6. W3C VC 2.0 + FHIR Immunization Modeling

### W3C VC Data Model 2.0 (W3C Recommendation, 15 May 2025)

Core: **`@context`** (must start with `https://www.w3.org/ns/credentials/v2`), **`type`** (must include `VerifiableCredential`), **`issuer`** (resolvable URI/object), **`credentialSubject`**, plus a securing mechanism (Data Integrity *or* JOSE/COSE). Recommended/optional: **`id`**, **`validFrom`** (renamed from v1.1 `issuanceDate`), **`validUntil`** (renamed from `expirationDate`), **`credentialStatus`** (revocation), **`credentialSchema`**, `name`, `description`. ([w3.org/TR/vc-data-model-2.0](https://www.w3.org/TR/vc-data-model-2.0/), [w3.org press release](https://www.w3.org/press-releases/2025/verifiable-credentials-2-0/))

> **xlsx bug:** the xlsx vaccine sheet puts a human sentence ("This is a verifiable credential that shows…") in the `@context` cell and the credential *title* in `type`. In a real VC, `@context` is an ordered set of context **URIs** and `type` is an array of **type tokens** (e.g. `["VerifiableCredential","RabiesVaccinationCredential"]`). The descriptive sentence belongs in `description`. The xlsx also lacks `credentialStatus` (no revocation) and `id`/`credentialSchema`.

### FHIR R4 Immunization resource — the human-health vaccine model

`status` (1..1), `vaccineCode` (1..1, **CodeableConcept**, CVX example-binding — allows multiple codings + free text fallback), `patient` (1..1), `occurrence[x]` (1..1, date administered), `recorded`, `primarySource`, `manufacturer` (Reference Organization), `lotNumber` (string), `expirationDate` (date), `site`, `route`, `doseQuantity` (Quantity with **value + unit**), `performer` (actor + function), `reaction`, **`protocolApplied`** (`targetDisease`, `doseNumber`, `seriesDoses`). ([hl7.org FHIR R4 Immunization](http://hl7.org/fhir/R4/immunization.html))

Key FHIR lessons for DogTag: vaccine is a **CodeableConcept** (a code *plus* a system *plus* optional text — never bare free text); **dose is a Quantity with an explicit unit**; **`protocolApplied.targetDisease` + dose-in-series** model the primary-vs-booster distinction that EU rabies rules hinge on; `manufacturer` is its own field (the xlsx folds manufacturer into the product-name string `"Nobivac/Merck"`).

### The veterinary vaccine coding gap — and the fix

CVX is human-only. **The veterinary analogue is the USDA APHIS Center for Veterinary Biologics Product Code (PCN)** — a CVB-assigned alphanumeric code on every licensed US veterinary biologic's license, alongside the manufacturer's Veterinary License Number (VLN) / Permittee Number (VPN). This is the standardized, machine-checkable identifier the DogTag vaccine record should carry instead of (or alongside) the free-text product name. ([aphis.usda.gov licensed products](https://www.aphis.usda.gov/veterinary-biologics/licensed-products), [APHIS Product Code Book PDF](https://www.aphis.usda.gov/animal_health/vet_biologics/publications/currentprodcodebook.pdf))

### The breed-taxonomy gap — and the fix

The xlsx breed value "**Golden Doodle**" is a **crossbreed that the FCI does not recognize and assigns no code** — FCI only registers ~356 purebreds. ([fci.be](https://www.fci.be/en/Nomenclature/)) The correct standard is the **Vertebrate Breed Ontology (VBO)** (Monarch Initiative + OMIA), which covers **19,500+ breed concepts across 49 species including mixed breeds/crossbreeds** with stable IDs — e.g. **`VBO:0200798` = Labradoodle**. VBO is the machine-readable breed coding the schema should use (with free-text fallback for true unknowns). ([Vertebrate Breed Ontology](https://monarch-initiative.github.io/vertebrate-breed-ontology/), [VBO paper, J Vet Intern Med 2025](https://onlinelibrary.wiley.com/doi/10.1111/jvim.70133))

---

## 7. Critical Analysis vs the DogTag xlsx Schema

### Modeling mistakes & gaps identified

| # | Issue in xlsx | Severity | Evidence |
|---|---|---|---|
| 1 | **No Owner entity at all.** Pet is the only subject; owner buried in nothing. | **Critical** | Every registry/passport/VEHCS models owner (consignor/consignee) as first-class; ownership transfer is the core lifecycle event (§2, §3). |
| 2 | **`Microchip ID` stored as a float** (`9.85e14`) → precision loss; only 12 of 15 digits survive. | **Critical** | ISO 11784 is a 15-digit string with leading-digit semantics; must be a fixed string (§2). |
| 3 | **`Weight` is a unitless scalar.** `50` lb? kg? | **High** | FHIR uses Quantity{value, unit}; apps store dated weight series (§1, §6). |
| 4 | **`Breed` free text "Golden Doodle"** — no taxonomy, a crossbreed FCI won't code. | **High** | Use VBO code + free-text label (§6). |
| 5 | **Vaccine product is free text; manufacturer folded into the name** (`"Nobivac/Merck"`). | **High** | FHIR/EU DCC split product, manufacturer, and use coded value sets; USDA PCN exists for vet vaccines (§5, §6). |
| 6 | **No `nextDueDate` for the vaccine.** | **High** | Both CDC and VEHCS rabies records require "date next due" (§3). |
| 7 | **No `microchipImplantDate`.** | **High** | EU + VEHCS enforce vax-must-not-precede-chip; cannot validate without it (§1, §3). |
| 8 | **`Service Dog (Y/N)` boolean** — no issuer, no task, no legal context. | **High** | Service-dog status is a *trust-tiered attestation*, not a flag; ESA≠service dog; DOJ rejects unverified flags (§4). |
| 9 | **`@context`/`type` misused** (prose in `@context`; title in `type`); no `credentialStatus`, no `id`, no `credentialSchema`. | **High** | VC 2.0 structural requirements; revocation must exist (§5, §6). |
| 10 | **`Age (at time of vax)` as free text** ("2 years, 5 months, 10 days."). | **Medium** | Store DOB once; derive age. Redundant + unparseable. |
| 11 | **`Sex` mixes sex + neuter status** ("Male Neutered"). | **Medium** | Split `sex` (enum) and `neuterStatus` (enum) — FHIR/passport practice (§3, §6). |
| 12 | **Pet identity duplicated onto every vaccine record** (name, breed, weight, microchip). | **Medium** | Reference the identity credential by ID, don't copy — copies drift and bloat the on-chain payload (§5 canonicalization). |
| 13 | **No `species` at DogProfile top level** (only on vaccine sheet); whole model is dog-only. | **Medium** | Registries/passports are multi-species (dog/cat/ferret) (§1, §3). |
| 14 | **No document/photo hash** for uploaded certs or the dog's photo (CDC requires a photo). | **Medium** | Anchor a hash of off-chain documents (§3, §5). |
| 15 | **No revocation / status mechanism** anywhere. | **High** | EU DCC's hardest retrofit — build it in from day one (§5). |

### Things the xlsx gets right (keep)
- W3C VC envelope for the vaccine credential (issuer/validFrom/validUntil) — correct direction, just mis-filled.
- Separate Issuer/Vet table with USDA accreditation # and license # — matches VEHCS/CDC reality.
- SBT framing for the dog identity (non-transferable identity token, transferable owner-binding) — aligns with §2's "identity shouldn't be tradable" and §5's SBT note.
- Microchip ID present at all — it is the universal cross-system key (§3).

---

## 8. Finalized Field Set (Recommendations per Record Type)

> **Conventions.** Types are logical. **R** = required, **O** = optional, **C** = conditional. Coded fields use `{ code, system, label }` (CodeableConcept pattern) so a canonical code anchors the hash and free text is display-only. Identity is referenced by ID from other records, never copied. On-chain stores **only the hash/Merkle root**; full records live off-chain (§5).

### 8.1 DogProfile (Identity SBT) — `credentialSubject`

| Field | Type | Req | Rationale / source |
|---|---|---|---|
| `dogTagId` | IRI/SBT id | R | Stable subject identifier |
| `species` | coded enum (`dog`/`cat`/`ferret`) | R | Multi-species; EU scope (§1, §3) |
| `name` | string | R | Universal |
| `breed` | `{ vboCode, label }` | R | **VBO code** + free-text label; covers crossbreeds (§6) |
| `sex` | enum(`male`,`female`) | R | Split from neuter (§7-#11) |
| `neuterStatus` | enum(`intact`,`neutered`,`spayed`,`unknown`) | O | Distinct from sex |
| `dateOfBirth` | date | R | Drives age + 12-week vax eligibility; **age is derived, never stored** (§7-#10) |
| `colorMarkings` | string | O→R-for-travel | Passport §II, CDC, VEHCS (§3) |
| `photoHash` | hash + URI | O→R-for-CDC | Hash of off-chain photo (§3) |
| `microchip` | object (see 8.2) | R | The cross-system key |
| `weightHistory` | array of `{ value, unit(`kg`/`lb`), measuredOn }` | O | Quantity w/ unit + date, not a scalar (§7-#3) |
| `currentOwner` | ref → Owner id | R | First-class owner (§7-#1) |
| `ownershipHistory` | array of `{ ownerId, from, to, transferProofHash }` | O | The real value of a chain registry (§2) |
| `serviceDogAttestation` | ref → ServiceDog credential | C | Only if applicable; not a boolean (§4) |
| `whitelist` / status | enum | R | Existing field, retain |

### 8.2 Microchip (sub-object)

| Field | Type | Req | Rationale |
|---|---|---|---|
| `code` | **string, 15 chars** | R | ISO 11784/11785; **never numeric** (§7-#2) |
| `standard` | enum(`ISO-11784/85-FDX-B`,`ISO-HDX`,`non-ISO`) | R | Legacy chips exist (§2) |
| `implantDate` | date | R | Required to validate vax-after-chip ordering (§3) |
| `bodyLocation` | string | O | Passport §III |
| `brand` | string | O | Registry field (§2) |

> Treat `code` as a **non-authoritative lookup key** (collisions/recycling possible) — uniqueness lives in the on-chain owner-binding, not the chip number (§2).

### 8.3 Owner (new entity — does not exist in xlsx)

| Field | Type | Req | Rationale |
|---|---|---|---|
| `ownerId` | IRI/DID | R | First-class, transferable subject (§2, §3) |
| `name` | string | R | Registry/passport §I |
| `address` | structured | R | Registry/passport/VEHCS consignor |
| `phone` | string[] | R | **Stale phone is the #1 recovery failure** (§2) |
| `email` | string | O | Registry/app matching |
| `emergencyContact` | object | O | Registry alt-contact (§2) |
| `contactUpdatedOn` | date | R | Surfaces staleness, the dominant failure mode (§2) |

### 8.4 Vaccine Credential (W3C VC 2.0)

Envelope: `@context` (URI array starting `https://www.w3.org/ns/credentials/v2`), `type` `["VerifiableCredential","RabiesVaccinationCredential"]`, `id`, `issuer` (vet DID), `validFrom`, `validUntil`, `credentialStatus` (revocation), `credentialSchema`, `description` (the prose that was wrongly in `@context`). `credentialSubject`:

| Field | Type | Req | Rationale / source |
|---|---|---|---|
| `dog` | ref → DogProfile id | R | **Reference, don't copy** identity (§7-#12) |
| `targetDisease` | coded (`Rabies`, system) | R | EU DCC `tg`; FHIR `protocolApplied.targetDisease` (§5, §6) |
| `vaccineProduct` | `{ usdaProductCode(PCN), label }` | R | **USDA APHIS PCN** = vet CVX analogue (§6) |
| `manufacturer` | `{ code/VLN, label }` | R | Own field; EU DCC `ma`; not folded into name (§7-#5) |
| `lotNumber` | string | R | EU/CDC mandate |
| `lotExpirationDate` | date | R | EU/CDC mandate |
| `vaccinationDate` | date | R | Must be ≥ microchip implant date, age ≥12 wk (§1, §3) |
| `validFrom` | date | R | = vaccinationDate + 21d for primary (see `01`) |
| `validUntil` | date | R | Per licence 1–3 yr |
| `nextDueDate` | date | R | **CDC/VEHCS require "date next due"** (§3, §7-#6) |
| `seriesType` | enum(`primary`,`booster`) | R | FHIR `protocolApplied`; drives 21-day rule (§6) |
| `route` / `site` | coded | O | FHIR fields |
| `doseQuantity` | `{ value, unit }` | O | FHIR Quantity (unit explicit) |
| `administeringVet` | ref → Vet id | R | Provenance |
| `documentHash` | hash + URI | O | Hash of off-chain paper cert (§5) |

> Pet identity attributes (name/breed/weight/microchip) that the xlsx duplicates here are **dropped** — resolved via the `dog` reference. Snapshot only `weightAtVaccination` if a clinical record demands it.

### 8.5 Vet / Issuer

| Field | Type | Req | Rationale |
|---|---|---|---|
| `issuerId` | DID | R | VC issuer identity |
| `name` | string | R | xlsx ✓ |
| `usdaAccreditationNumber` | string (6-digit NAN) | R | VEHCS/CDC; NVAP (§3, `01`) |
| `accreditationCategory` | enum(`I`,`II`) | O | CDC form field (§3) |
| `stateLicenseNumber` | string | R | xlsx "local license number" |
| `licenseState` | string | R | VEHCS state-authorization (§3) |
| `hospital` / `clinic` | string | R | xlsx ✓ |
| `address` | structured | R | xlsx ✓ |
| `phone` | string | R | xlsx ✓ |
| `keyDirectoryEntry` | URI | R | VCI-style issuer key for offline verify (§5) |
| `status` / whitelist | enum | R | Revocable issuer trust |

### 8.6 ServiceDog Attestation (replaces the `Y/N` boolean)

| Field | Type | Req | Rationale |
|---|---|---|---|
| `subjectDog` | ref → DogProfile | R | |
| `issuerTrustTier` | enum(`adi-accredited-org`,`handler-self-attestation`,`other`) | R | Trust value is issuer-determined (§4) |
| `issuer` | ref (org DID *or* handler DID) | R | ADI org > handler self-attestation >> registry |
| `taskDescription` | string | R | ADA defines service dog by **task**, not paper (§4) |
| `legalContext` | enum[`ada-public-access`,`acaa-air`,`fha-housing`] | R | Same animal, different status per law (§4) |
| `category` | enum(`service-dog`,`psychiatric-service-dog`,`esa`) | R | ESA is housing-only, a pet for air/ADA (§4) |
| `dotFormRef` | document hash | C | For ACAA air travel; handler self-attestation under 18 U.S.C. §1001 (§4) |
| `rabiesCredentialRef` | ref → Vaccine VC | C | DOT form requires rabies attestation (§4) |
| `validUntil` / `credentialStatus` | — | R | Revocable |

> **Explicitly do NOT model a "buy-a-certificate" registry issuer** — DOJ deems it worthless; on-chain it would be a permanently-immutable worthless claim (§4, §5).

---

## 9. Source Index (all cited URLs)

**Apps:** petdesk.com/download-app-for-pet-health · petdesk.zendesk.com/.../360052833813 · 11pets.com/en/feature · 11pets.com/en/pet-care-app · play.google.com/.../com.m11pets.elevenpets
**Microchip:** en.wikipedia.org/wiki/ISO_11784_and_ISO_11785 · maxmicrochip.com/ISO_types.htm · service-icar.com/tables/Tabella3.php · rfidnews.com/ISOstandard/ISOstandard.html · intercom.help/animal-id-corporation/.../10498528 · peeva.co/blog/strategic-incompatibility-a-timeline · avma.org/.../microchipping-faq · akcreunite.org/microchippingmypetfaq · akc.org/products-services/akc-reunite · classaction.org/news/home-again-deceives-pet-owners · foundanimals.org/microchip-register-faqs · foundanimals.org/aaha-universal-pet-microchip-lookup-tool · aaha.org/.../microchip-search · petlog.org.uk/.../compulsory-cat-microchipping
**Travel:** eur-lex.europa.eu/eli/reg_impl/2026/705/oj · eur-lex.europa.eu/.../CELEX:32013R0577 · food.ec.europa.eu/.../travelling-pet-within-eu_en · daera-ni.gov.uk/.../PT1...pdf · aphis.usda.gov/pet-travel/vehcs · vehcs-training.aphis.usda.gov/VEHCSHelp/VEHCS_Create_Certificate.pdf · cdc.gov/importation/dogs/dog-import-form-instructions.html · cdc.gov/importation/dogs/dog-importation-navigator.html · cdc.gov/importation/hcp/dog-importation/instructions-us-issued-rabies-vaccination-form.html · united.com/.../savedpets · bringfido.com/privacy · padspass.com/digitalpetpassport · food.ec.europa.eu/.../agri-fraud_report_Illegal-trade_cats-dogs.pdf · four-paws.org/.../illegal-puppy-trade
**Service dog:** ada.gov/resources/service-animals-faqs · ada.gov/resources/service-animals-2010-requirements · law.cornell.edu/cfr/text/14/382.75 · transportation.gov/.../traveling-air-service-animals · assistancedogsinternational.org/standards/what-is-accreditation
**Blockchain/VC:** github.com/ehn-dcc-development/eu-dcc-schema · github.com/ehn-dcc-development/eu-dcc-valuesets · github.com/ehn-dcc-development/eu-dcc-hcert-spec · eur-lex.europa.eu/.../CELEX:32021D1073 · spec.smarthealth.cards · github.com/smart-on-fhir/health-cards · coinmarketcap.com/currencies/pawtocol · beincrypto.com/how-agriculture-focused-blockchain-products-are-actually-doing-today · rfc-editor.org/rfc/rfc8785 · w3.org/TR/rdf-canon · rfc-editor.org/rfc/rfc8949
**VC2.0/FHIR/coding:** w3.org/TR/vc-data-model-2.0 · w3.org/press-releases/2025/verifiable-credentials-2-0 · hl7.org/fhir/R4/immunization.html · aphis.usda.gov/veterinary-biologics/licensed-products · aphis.usda.gov/animal_health/vet_biologics/publications/currentprodcodebook.pdf · fci.be/en/Nomenclature · monarch-initiative.github.io/vertebrate-breed-ontology · onlinelibrary.wiley.com/doi/10.1111/jvim.70133

### Sourcing caveats
- AKC Reunite / transportation.gov / major airline pages bot-block automated fetch; their field lists are corroborated from FAQs, eCFR, official PDFs, and multiple independent walkthroughs.
- EU Pet Passport field labels come from official completion guidance (renders as image in the regulation); pixel-exact spec needs manual transcription.
- USDA fee dollar amounts and some airline fees/dimensions are from 2026 third-party guides; the regulatory increases are confirmed via Federal Register but reconfirm exact figures before hard-coding.
- EU DCC value-set membership shifts slightly between dated releases (read from release 1.3.0 / main).

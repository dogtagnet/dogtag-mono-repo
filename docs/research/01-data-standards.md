# DogTag — Data Standards Brief (Pet Identity, Vaccination & Cross-Border Travel)

> **Purpose.** Authoritative, citation-backed reference for designing open, verifiable-credential (VC) data schemas covering pet identity, rabies vaccination, and cross-border travel documents.
> **Scope.** EU non-commercial pet movement, rabies vaccination certificates, USDA APHIS endorsement, the 2024 CDC Dog Import Form, the US DOT Service Animal Air Transportation Form, the W3C VC Data Model v2.0 envelope, and ISO 11784/11785 microchip identifiers.
> **Date of research.** 2026-06-17. Regulatory citations note that the EU 2013 framework has been substantially restated by 2019/2026 delegated/implementing acts (see notes).
> **Convention.** "MANDATED" = a field that a cited real-world standard explicitly requires. "Derived" = a field we propose for schema completeness that is implied but not verbatim-required.

---

## 1. EU Non-Commercial Pet Movement (Reg (EU) No 576/2013 & 577/2013)

### Legal framework
- **Regulation (EU) No 576/2013** — the basic act governing non-commercial movement of pet animals (dogs, cats, ferrets). Sets identification, rabies vaccination, and health-document rules.
- **Implementing Regulation (EU) No 577/2013** — model documents: the **EU pet passport** (intra-EU and from listed/Annex I countries) and the **Annex IV animal health certificate** (entry from non-listed/third countries).
- **Commission Implementing Regulation (EU) 2016/561** amended Annex IV (the model AHC for dogs/cats/ferrets). The 2013 framework has since been restated/updated by later acts (Delegated Reg (EU) 2019/2035 Art. 70a on transponder specs; and 2026-series delegated/implementing acts cited by the Commission portal). Validate exact article numbers against current EUR-Lex before production use.

Sources: [food.ec.europa.eu – bringing a pet into the EU](https://food.ec.europa.eu/animals/live-animal-movements/dogs-cats-and-ferrets/bringing-pet-eu-non-eu-country_en) · [food.ec.europa.eu – travelling within the EU](https://food.ec.europa.eu/animals/live-animal-movements/dogs-cats-and-ferrets/travelling-pet-within-eu_en) · [food.ec.europa.eu – non-commercial movement from non-EU countries](https://food.ec.europa.eu/animals/movement-pets/eu-legislation/non-commercial-movement-non-eu-countries_en) · [legislation.gov.uk – Reg (EU) 2016/561](https://www.legislation.gov.uk/eur/2016/561/2020-01-31/data.xht?view=snippet&wrap=true)

### Two document types
| Document | When used | Issued by |
|---|---|---|
| **EU Pet Passport** | Movement within the EU, and entry from Annex I "listed" third countries | An authorised veterinarian |
| **Annex IV Animal Health Certificate (AHC)** | Entry from non-listed third countries (e.g. from the US) | Official/authorised veterinarian; **USDA-endorsed** when exported from the US (see §3) |

### Key mandated rules (verbatim thresholds)
- **Identification (MANDATED):** microchip transponder implantation per ISO 11784/11785; or a clearly readable **tattoo applied before 3 July 2011**. (Reg 576/2013 Art. 17 / Annex II; transponder tech per Delegated Reg (EU) 2019/2035 Art. 70a.)
- **Microchip-before-vaccination ordering (MANDATED):** "The date of administration of the vaccine does **not precede** the date of identification or reading of the microchip."
- **Rabies vaccination age (MANDATED):** the animal "was at least **12 weeks old** at the date the vaccine was administered."
- **Vaccination validity start (MANDATED):** validity "starts **not less than 21 days** from the completion of the vaccination protocol for the primary vaccination." Boosters must be given "within the period of validity of the preceding vaccination" (no 21-day re-wait if continuous).
- **Rabies antibody titration test (MANDATED for entry from non-listed third countries):** neutralising antibody level **≥ 0.5 IU/ml**; blood sample taken **at least 30 days after** the primary vaccination and **not less than 3 months (90 days) before** the date of issue of the certificate; performed by an **EU-approved/designated laboratory**. The 3-month wait is waived for re-entry of a previously tested pet with maintained vaccination.
- **Echinococcus multilocularis (tapeworm) treatment (MANDATED for dogs entering FI, IE, MT, NO):** praziquantel or equivalent, administered by a vet **between 24 and 120 hours before** scheduled entry. Cats/ferrets exempt.
- **Certificate validity (MANDATED):** the AHC is **valid for 10 days** from date of issue until documentary/identity checks at entry (extended by sea-travel duration); valid for **4 months** for onward movement within the EU thereafter (or until the rabies vaccination expires, whichever is sooner).

Sources: [food.ec.europa.eu – non-commercial movement from non-EU countries](https://food.ec.europa.eu/animals/movement-pets/eu-legislation/non-commercial-movement-non-eu-countries_en) · [food.ec.europa.eu – travelling within the EU](https://food.ec.europa.eu/animals/live-animal-movements/dogs-cats-and-ferrets/travelling-pet-within-eu_en) · [APHIS – EU non-commercial HC guidance (PDF)](https://www.aphis.usda.gov/sites/default/files/eu-noncommercial-hc-first-page-guidance.pdf)

### Annex IV AHC / Pet Passport field sections (per Reg 577/2013 model)
The certificate/passport documents: the **transponder or tattoo alpha-numeric code**, **rabies vaccination details**, **blood-sampling (titration) details**, and where applicable **Echinococcus treatment details**. The standard passport sections (Annex III, Reg 577/2013) are: I Details of ownership · II Description of animal · III Marking of animal · IV Issuing of the passport · V Vaccination against rabies · VI Rabies antibody titration test · VII Echinococcus / parasite treatment · VIII–XI Clinical exam / other vaccinations / legalisation (only as required by destination) · XII Others. (Note: the rabies-vaccination block is **§V**; the field-list cross-references below use this numbering.)

### Proposed field list — **EU Pet Travel Credential** (passport + AHC superset)
| Field | Type | Req | Notes |
|---|---|---|---|
| `species` | enum(`dog`,`cat`,`ferret`) | MANDATED | Scope of the scheme |
| `breed` | string | MANDATED | Description of animal §II |
| `sex` | enum(`male`,`female`) | MANDATED | |
| `dateOfBirth` | date | MANDATED | Drives 12-week vaccination eligibility |
| `colour` | string | MANDATED | |
| `notableFeatures` | string | Optional | Markings/features |
| `transponderCode` | string (15 num) | MANDATED | ISO 11784/11785; see §7 |
| `transponderApplicationDate` | date | MANDATED | Must be ≤ vaccination date |
| `transponderReadingDate` | date | Optional | If re-read |
| `transponderLocation` | string | MANDATED | Body location of chip |
| `tattooCode` | string | Conditional | Only valid if applied before 2011-07-03 |
| `ownerName` | string | MANDATED | §I |
| `ownerAddress` | string | MANDATED | §I |
| `rabiesVaccine` (object) | see §2 | MANDATED | Passport §V |
| `rabiesTitrationTest` (object) | object | Conditional | Passport §VI; MANDATED for non-listed third countries |
| `echinococcusTreatment` (object) | object | Conditional | Passport §VII; MANDATED for dogs → FI/IE/MT/NO |
| `certificateValidFrom` | dateTime | MANDATED | Issue date |
| `certificateValidUntil` | dateTime | MANDATED | Issue + 10 days for entry |
| `issuingVeterinarian` | object{name,address,signature,officialStamp} | MANDATED | Official/authorised vet |
| `usdaEndorsement` | object | Conditional | When exported from US — see §3 |

`rabiesTitrationTest` object: `{ bloodSamplingDate: date, resultIU: number (≥0.5), laboratoryName: string, approvedLab: boolean }`.
`echinococcusTreatment` object: `{ productName: string, manufacturer: string, treatmentDateTime: dateTime, administeringVet: object }`.

---

## 2. Rabies Vaccination Certificate Fields

The rabies vaccination block (EU pet passport §IV and AHC) **mandates** the following fields; omission of manufacturer, product name, or batch number is treated as a **non-compliance**:
- **Manufacturer** of the vaccine
- **Name / product name** of the vaccine
- **Batch / lot number**
- **Date of administration** (vaccination date)
- **Valid from** (modern passport model added an explicit "valid from" — i.e. administration date + 21 days for a primary series)
- **Valid until** (expiry; set per the vaccine's licensed duration, typically 1–3 years)
- **Authorised veterinarian** name, full contact details, and signature

Validity-window rules (see §1): valid-from = **21 days after** completion of the **primary** protocol; boosters extend validity without a fresh 21-day wait **only if** given before the prior dose expires. If a booster lapses, the animal is treated as a primary vaccination again (21-day wait re-applies).

Sources: [mapa.gob.es – travelling with dogs/cats/ferrets FAQ](https://www.mapa.gob.es/en/ganaderia/preguntas-frecuentes/FAQS-TRAVEL-PETS.aspx) · [food.ec.europa.eu – travelling within the EU](https://food.ec.europa.eu/animals/live-animal-movements/dogs-cats-and-ferrets/travelling-pet-within-eu_en)

### Proposed field list — **Rabies Vaccination Certificate (credentialSubject)**
| Field | Type | Req | Notes |
|---|---|---|---|
| `vaccineProductName` | string | MANDATED | EU non-compliance if missing |
| `vaccineManufacturer` | string | MANDATED | EU non-compliance if missing |
| `batchNumber` | string | MANDATED | EU non-compliance if missing |
| `vaccinationDate` | date | MANDATED | Must be after microchip date & age ≥12 wks |
| `validFrom` | date | MANDATED | = vaccinationDate + 21d for primary series |
| `validUntil` | date | MANDATED | Per vaccine licence (1–3 yr) |
| `vaccinationType` | enum(`primary`,`booster`) | Derived | Drives 21-day rule |
| `administeringVeterinarian` | object{name,contact,accreditationNo,signature} | MANDATED | |
| `animalReference` | string/IRI | Derived | Links to identity credential (transponderCode) |

---

## 3. USDA APHIS Endorsement of EU Health Certificates (US-exported pets)

For pets exported from the US to the EU, the **Annex IV AHC must be issued (completed and signed) by a USDA-Accredited Veterinarian and then endorsed (counter-signed and embossed/stamped) by APHIS** before departure.

- **USDA Accreditation / National Accreditation Number (NAN):** Each accredited veterinarian is assigned a **six-digit National Accreditation Number (NAN)** under the National Veterinary Accreditation Program (NVAP, run by APHIS Veterinary Services). The NAN identifies the certifying vet; note a NAN does **not** by itself authorise nationwide practice — the vet must be state-authorised by the local APHIS Veterinary Medical Officer.
- **Endorsement process:** The accredited vet completes/signs/dates the certificate and submits it to the **USDA Endorsement Office via the Veterinary Export Health Certification System (VEHCS)** (or provides paperwork to the owner to submit). APHIS reviews, counter-signs, and embosses/stamps it.
- **Timing constraint:** The certificate must be **endorsed and the pet must enter the EU within 10 days** of issue/endorsement (mirrors the §1 10-day AHC validity).

Sources: [APHIS – Pet Passports, European Union](https://www.aphis.usda.gov/pet-travel/another-country-to-us-import/pet-passports-european-union) · [APHIS – Pet travel process overview](https://www.aphis.usda.gov/pet-travel/pet-travel-process-overview) · [APHIS NVAP – National Accreditation Number](https://www.aphis.usda.gov/nvap/accred-number) · [APHIS – Accredited Veterinarians: certifying for export](https://www.aphis.usda.gov/pet-travel/accredited-veterinarians) · [pettravel.com – USDA endorsement](https://www.pettravel.com/passports_USDA_certification.cfm)

### Proposed field list — **USDA Endorsement (sub-object on a travel credential)**
| Field | Type | Req | Notes |
|---|---|---|---|
| `accreditedVetName` | string | MANDATED | Issuing vet |
| `nationalAccreditationNumber` | string (6 digits) | MANDATED | NAN |
| `vetStateAuthorization` | string | Derived | State in which vet is authorised |
| `vehcsSubmissionId` | string | Derived | VEHCS reference |
| `endorsementOffice` | string | Derived | APHIS Endorsement Office |
| `endorsementDate` | date | MANDATED | Starts the 10-day window |
| `endorsementStamp` | binary/hash | MANDATED | Embossed/stamped seal (digitised) |

---

## 4. CDC Dog Import Form (effective 1 Aug 2024)

Effective **August 1, 2024**, CDC requires all dogs entering or re-entering the US to meet new rules and the importer to submit the **CDC Dog Import Form** online before travel.

Mandated requirements that apply to **all** dogs (verifiable at the border, not all captured on the online form):
- **Microchip (MANDATED, physical):** every dog must have an **ISO-compatible microchip readable by a universal scanner**. The microchip **must be implanted before the qualifying rabies vaccination**. (CBP/CDC can scan on arrival.)
- **Minimum age (MANDATED):** the dog must be **at least 6 months old** at the time of entry/return.
- **Appear healthy on arrival (MANDATED).**
- **Receipt (MANDATED to present):** after submission and email verification, CDC emails a **CDC Dog Import Form receipt within ~15 minutes**. Per CDC: it is *"valid for one dog to enter the United States multiple times from the same country within six months from the date of issuance"* and **must be shown to the airline (if flying) and to CBP on arrival**. Validity is broken if the dog visits a high-risk or different country.

> **Gotcha — the form's field set is pathway-dependent.** For dogs arriving from **dog-rabies-free or low-risk countries**, the online form's **Section B does NOT collect a microchip number or a dog photo** — it asks only for name, age, sex, breed, and colour/markings. The **microchip number and a face-and-body photo are collected/required on the form only for the high-risk-country pathway** ("Enter the microchip number of the dog"; "Attach a photo of the dog showing its face and body"). The universal-scanner-readable microchip is still *physically* required for every dog; it is just not a form field on the low-risk path. Schema must therefore treat `microchipNumber`/`photo` as **conditionally required** (required when `wasInHighRiskCountry = true`), not unconditionally. (Source: CDC Dog Import Form instructions, pathway sections.)

Form data fields (low-risk path): dog name, breed (or "mixed/other"), colour & markings, sex, age/DOB; importer full name, DOB, email, phone, ID (passport / driver's licence / air waybill); country of departure, intended US arrival date, arrival airport/port, travel mode, and whether the dog was in a **high-risk rabies country** within 6 months. The **high-risk path additionally requires** the microchip number, a face-and-body photo (.jpg/.jpeg/.png, ≤10 MB; dogs <1 yr: taken within 15 days before arrival), and triggers further requirements (e.g. CDC import permit / Certification of Foreign Rabies Vaccination & Microchip, reservation at a CDC-registered animal care facility, serologic titer from a CDC-approved lab).

Sources: [CDC – Dog Import Form & instructions](https://www.cdc.gov/importation/dogs/dog-import-form-instructions.html) · [CDC – entry from dog-rabies-free/low-risk countries](https://www.cdc.gov/importation/dogs/rabies-free-low-risk-countries.html) · [CDC Newsroom – updated import process (22 Jul 2024)](https://www.cdc.gov/media/releases/2024/s0722-dog-importation.html) · [AVMA – CDC import FAQs for veterinarians](https://www.avma.org/resources-tools/animal-health-and-welfare/animal-travel-and-transport/cdc-dog-importation-requirements-faqs-veterinarians)

### Proposed field list — **CDC Dog Import Credential**
| Field | Type | Req | Notes |
|---|---|---|---|
| `dogName` | string | MANDATED | |
| `microchipNumber` | string (15 num) | Conditional (MANDATED on high-risk path; physical chip required for all) | ISO-compatible; implanted before rabies vax. Not a low-risk-path form field. |
| `breed` | string | MANDATED | "mixed/other" allowed |
| `colorMarkings` | string | MANDATED | |
| `sex` | enum | MANDATED | |
| `ageOrDOB` | date/int | MANDATED | ≥ 6 months at entry |
| `photo` | image ref | Conditional (high-risk path) | ≤10MB; <1yr → within 15 days |
| `importerName` | string | MANDATED | |
| `importerDOB` | date | MANDATED | |
| `importerEmail` | string | MANDATED | Used to verify + receive receipt |
| `importerPhone` | string | MANDATED | |
| `importerId` | string | MANDATED | passport / DL / air waybill no. |
| `countryOfDeparture` | string | MANDATED | |
| `countriesVisited6mo` | array<string> | MANDATED | |
| `wasInHighRiskCountry` | boolean | MANDATED | Triggers extra requirements |
| `arrivalDate` | date | MANDATED | |
| `arrivalPort` | string | MANDATED | |
| `travelMode` | enum(`air`,`land`,`sea`) | MANDATED | |
| `receiptId` | string | MANDATED | CDC-issued; valid 6 months |
| `receiptValidUntil` | date | Derived | issue + 6 months |

---

## 5. US DOT Service Animal Air Transportation Form

A US Department of Transportation form (under 14 CFR Part 382) that US airlines **may require** passengers travelling with a service animal to submit. An airline may require it **up to 48 hours in advance** if the reservation was booked >48h before departure (otherwise at the gate).

Sections / fields (latest 2024 form):
- **A — Handler & User info (MANDATED):** handler name, user/passenger name (may differ), address, phone, email.
- **B — Service animal identification & health (MANDATED):** animal name, type/species, breed, weight, colour; **rabies vaccination expiration date (month/day/year)**; attestation the animal is **free of fleas, ticks, and disease and is vaccinated for rabies**.
- **C — Training (MANDATED):** name and phone of the person/organisation that trained the animal (handler may self-list); description of the task(s) the animal performs.
- **D — Behaviour attestation (MANDATED):** that the animal behaves properly in public (no biting/barking/lunging, no relieving itself in cabin/gate).
- **Assurances + perjury / signature block (MANDATED):** acknowledgement of leash/harness control and liability; **signature and date under penalty of perjury**.

> **Trust-model note (important for VC design).** Unlike the EU/USDA documents, the DOT form is **self-attestation by the handler under penalty of perjury (18 U.S.C. §1001)** — not an authority- or vet-issued credential. In VC terms the *issuer is the handler* (a self-asserted/holder-issued credential), whereas rabies/EU/CDC documents are issued by a vet or government authority. Model accordingly: do not treat DOT attestations as third-party-verified facts.

Sources: [DOT – Service Animal Air Transportation Form (sample page)](https://www.transportation.gov/individuals/aviation-consumer-protection/service-animals/Air_Transportation_Form) · [DOT – Service Animal Air Transportation Form FINAL 9.20.24 (PDF)](https://www.transportation.gov/sites/dot.gov/files/2024-09/Service%20Animal%20-%20Air%20Transportation%20Form%20FINAL%209.20.24.pdf) · [DOT – Service Animals resource page](https://www.transportation.gov/resources/individuals/aviation-consumer-protection/service-animals) · [esadoctors.com – how to fill out the DOT form](https://esadoctors.com/dot-service-animal-air-transportation-form/)

### Proposed field list — **DOT Service Animal Credential**
| Field | Type | Req | Notes |
|---|---|---|---|
| `handlerName` | string | MANDATED | §A |
| `userName` | string | MANDATED | §A; may equal handler |
| `address` | string | MANDATED | §A |
| `phone` | string | MANDATED | §A |
| `email` | string | MANDATED | §A |
| `animalName` | string | MANDATED | §B |
| `animalType` | string | MANDATED | §B (dog; psychiatric service dog allowed) |
| `breed` | string | MANDATED | §B |
| `weight` | number | MANDATED | §B |
| `color` | string | MANDATED | §B |
| `rabiesVaxExpiration` | date | MANDATED | §B (mm/dd/yyyy) |
| `healthAttestation` | boolean | MANDATED | §B free of fleas/ticks/disease |
| `trainerName` | string | MANDATED | §C |
| `trainerPhone` | string | MANDATED | §C |
| `taskDescription` | string | MANDATED | §C |
| `behaviorAttestation` | boolean | MANDATED | §D |
| `assurancesAck` | boolean | MANDATED | leash/liability |
| `handlerSignature` | signature | MANDATED | under penalty of perjury |
| `signatureDate` | date | MANDATED | |

---

## 6. W3C Verifiable Credentials Data Model v2.0 (the credential envelope)

The VC Data Model v2.0 became a **W3C Recommendation on 15 May 2025**. It defines the extensible JSON-LD data model and a three-party ecosystem (issuer → holder → verifier). The DogTag Rabies Vaccination Certificate VC (and all DogTag credentials) should wrap their domain fields inside this canonical envelope.

### Canonical top-level properties
| Property | Type | Req | Notes |
|---|---|---|---|
| `@context` | ordered array | **Required** | First item **must** be `https://www.w3.org/ns/credentials/v2`; subsequent items are URLs/context objects (add a DogTag vocab context) |
| `type` | array of strings | **Required** | Must include `"VerifiableCredential"` plus a specific type, e.g. `"RabiesVaccinationCertificate"` |
| `issuer` | URL **or** object with `id` | **Required** | The vet/clinic/authority DID or URL; object form carries `name`, etc. |
| `credentialSubject` | object (or array) | **Required** | The claims — i.e. the domain field set (the pet, the vaccination) |
| `id` | single URL | Optional | Credential identifier |
| `validFrom` | `xsd:dateTime` | Optional | When the credential becomes valid (**renamed from v1.1 `issuanceDate`**) — map rabies `validFrom` here |
| `validUntil` | `xsd:dateTime` | Optional | Expiry (**renamed from v1.1 `expirationDate`**) — map rabies `validUntil` here |
| `credentialStatus` | object | Optional | Revocation/suspension (e.g. status-list) — useful for revoked vaccinations |
| `credentialSchema` | object/array | Optional | Reference to the JSON Schema validating `credentialSubject` |
| `proof` | object/array | Conditional | Required to make it a **secured** VC (Data Integrity / JOSE-COSE); not part of the unsecured core |

### v1.1 → v2.0 changes relevant to us
- `issuanceDate` → **`validFrom`**; `expirationDate` → **`validUntil`** (clearer validity-window semantics — maps perfectly onto rabies `valid from` / `valid until`).
- Base context URL changed to `https://www.w3.org/ns/credentials/v2`.

### Minimal envelope example (Rabies Vaccination Certificate VC)
```json
{
  "@context": [
    "https://www.w3.org/ns/credentials/v2",
    "https://dogtag.example/vocab/v1"
  ],
  "type": ["VerifiableCredential", "RabiesVaccinationCertificate"],
  "issuer": { "id": "did:web:vetclinic.example", "name": "Example Veterinary Clinic" },
  "validFrom": "2026-03-21T00:00:00Z",
  "validUntil": "2029-02-28T00:00:00Z",
  "credentialSubject": {
    "id": "did:example:pet-transponder-826098100212345",
    "transponderCode": "826098100212345",
    "vaccineProductName": "Nobivac Rabies",
    "vaccineManufacturer": "MSD Animal Health",
    "batchNumber": "A123B",
    "vaccinationDate": "2026-02-28",
    "vaccinationType": "primary"
  }
}
```
> Design note: the rabies table's `validFrom`/`validUntil` should live on the **envelope** (not duplicated in `credentialSubject`) so verifiers can evaluate validity generically. The vaccine product/manufacturer/batch/date stay inside `credentialSubject`.

Sources: [W3C – Verifiable Credentials Data Model v2.0 (Recommendation)](https://www.w3.org/TR/vc-data-model-2.0/) · [W3C – VC 2.0 publication history](https://www.w3.org/standards/history/vc-data-model-2.0/) · [W3C press release – VC 2.0 standard (May 2025)](https://www.w3.org/press-releases/2025/verifiable-credentials-2-0/)

---

## 7. Microchip ID Standards (ISO 11784 / ISO 11785)

Pet identification across the EU and the 2024 CDC rules relies on **ISO-standard RFID transponders**.

- **ISO 11784** — defines the **structure / code** of the radio-frequency identification of animals: a **15-digit decimal code**. The leading digits encode either a **3-digit ISO 3166 country code** (national identification scheme) **or** a **manufacturer code in the 900–999 range** (assigned by ICAR), followed by a unique **national/animal identification number** that fills the remaining digits. (Bit 27 / the "animal application" indicator and a manufacturer/country flag distinguish the two schemes internally.)
- **ISO 11785** — defines the **technical concept**: how the transponder is activated and how data is transferred. Operates at **134.2 kHz**, and supports both **FDX-B** (full-duplex) and **HDX** (half-duplex) protocols. A "universal" / ISO-compliant scanner reads both.

### Compliance gotchas (load-bearing for the schema)
- **EU (Reg 576/2013):** the transponder must comply with **ISO 11784 and Annex A of ISO 11785**. If a pet has a non-ISO chip, the **owner must provide a compatible reader** at checks.
- **CDC (eff. 1 Aug 2024):** dogs must have a microchip **readable by a universal scanner**; CDC accepts ISO-compatible chips and records the number on the import form. The chip **must be implanted before the qualifying rabies vaccination**.
- **"900"-prefix duplicates:** manufacturer-code (900-range) chips are not globally guaranteed unique the way country-code chips are; some 900-range numbers have historically been duplicated. Schema should treat the 15-digit code as the canonical identifier but allow recording the scheme (country vs manufacturer) for disambiguation.
- **Format:** exactly **15 numeric digits**, no letters, no separators. Validate with `^[0-9]{15}$`.

Sources: [ISO – ISO 11784:1996 Radio-frequency identification of animals – Code structure](https://www.iso.org/standard/19951.html) · [ISO – ISO 11785:1996 Technical concept](https://www.iso.org/standard/19982.html) · [AVMA – microchipping FAQ (ISO standard / universal scanners)](https://www.avma.org/resources-tools/pet-owners/petcare/microchipping-animals-faq) · [CDC – Dog Import Form & instructions](https://www.cdc.gov/importation/dogs/dog-import-form-instructions.html) · [food.ec.europa.eu – non-commercial movement](https://food.ec.europa.eu/animals/movement-pets/eu-legislation/non-commercial-movement-non-eu-countries_en)

### Proposed field list — **Microchip / Transponder (credentialSubject of a Pet Identity Credential)**
| Field | Type | Req | Notes |
|---|---|---|---|
| `transponderCode` | string, `^[0-9]{15}$` | MANDATED | The 15-digit ISO 11784 code; canonical pet ID |
| `codingScheme` | enum(`country`,`manufacturer`) | Derived | country=ISO 3166 prefix; manufacturer=900–999 (ICAR) |
| `isoCompliant` | boolean | MANDATED (effect) | True if ISO 11784/11785; if false, EU/CDC require owner-supplied reader |
| `protocol` | enum(`FDX-B`,`HDX`) | Optional | ISO 11785 air-interface |
| `applicationDate` | date | MANDATED (EU) | Must be ≤ rabies vaccination date |
| `readingDate` | date | Optional | When re-scanned at checks |
| `implantLocation` | string | MANDATED (EU) | Body location of the chip |
| `tattooCode` | string | Conditional | Legacy ID; valid only if applied before 2011-07-03 |

---

## Cross-cutting schema decisions (summary)

1. **One envelope, many subjects.** Wrap every DogTag document in the W3C VC v2.0 envelope; put domain fields in `credentialSubject`; map all validity windows onto envelope `validFrom`/`validUntil`.
2. **Transponder code is the join key.** The 15-digit ISO 11784 code links Pet Identity → Rabies Vaccination → EU Travel → CDC Import credentials.
3. **Encode ordering constraints.** Microchip date ≤ vaccination date; vaccination age ≥ 12 weeks (EU); these are real-standard invariants, enforce in schema/validation, not just UI.
4. **Validity windows are computed, not free-text.** EU rabies `validFrom` = vaccination date + 21 days (primary); titration result ≥ 0.5 IU/ml; AHC valid 10 days; CDC receipt valid 6 months.
5. **Distinguish trust models.** Vet/authority-issued credentials (rabies, EU AHC/passport, USDA endorsement, CDC) vs. **handler self-attestation** (DOT service-animal form, under penalty of perjury). The DOT credential's issuer is the holder — verifiers should weight it differently.
6. **Version the legal basis.** The 2013 EU framework (576/2013, 577/2013) has been progressively restated/recodified by later delegated and implementing acts (e.g. Delegated Reg (EU) 2019/2035 Art. 70a on transponders, and the 2026/131-series acts cited on the Commission portal). Store the citation/version on each schema and re-verify article numbers against EUR-Lex before production.


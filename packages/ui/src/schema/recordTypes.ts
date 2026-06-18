/**
 * Schema-driven issue-form field definitions (impl §1.6 / §11.5). These describe the
 * `fields` payload sent to POST /credentials/prepare — the backend builds the VC and
 * validates per §1.6, so these mirror the required field set + coded values.
 *
 * This is intentionally a UI-form descriptor (not the full VC). `dogTagId` is collected
 * separately (top-level on the prepare request). Nested fields use dotted paths
 * (e.g. "microchip.code") and are reassembled into a nested object before submit.
 */

export type FieldKind = "text" | "number" | "date" | "select" | "textarea";

export interface FieldDef {
  /** dotted path within `fields` (e.g. "microchip.code") */
  path: string;
  label: string;
  kind: FieldKind;
  required?: boolean;
  placeholder?: string;
  help?: string;
  options?: { value: string; label: string }[];
  /** simple client-side validators (see validateField) */
  pattern?: RegExp;
  patternMessage?: string;
}

export interface RecordTypeSchema {
  /** recordType key sent to the backend (also the `type` array entry) */
  recordType: string;
  label: string;
  description: string;
  groups: { title: string; fields: FieldDef[] }[];
}

const MICROCHIP_FIELDS: FieldDef[] = [
  {
    path: "microchip.code",
    label: "Microchip code",
    kind: "text",
    required: true,
    placeholder: "15 digits",
    pattern: /^[0-9]{15}$/,
    patternMessage: "Must be exactly 15 digits",
  },
  {
    path: "microchip.standard",
    label: "Microchip standard",
    kind: "select",
    required: true,
    options: [
      { value: "ISO_11784_11785", label: "ISO 11784/11785" },
      { value: "OTHER", label: "Other" },
    ],
  },
  {
    path: "microchip.implantDate",
    label: "Implant date",
    kind: "date",
    required: true,
    help: "Must be on or before the vaccination date",
  },
];

export const RABIES_VACCINATION: RecordTypeSchema = {
  recordType: "RabiesVaccinationCertificate",
  label: "Rabies Vaccination Certificate",
  description: "USDA-coded rabies vaccination credential (microchip mandatory).",
  groups: [
    { title: "Microchip", fields: MICROCHIP_FIELDS },
    {
      title: "Vaccine",
      fields: [
        {
          path: "vaccineProductCode",
          label: "Vaccine product code (USDA APHIS PCN)",
          kind: "text",
          required: true,
          placeholder: "e.g. 1351.20",
        },
        { path: "vaccineProductName", label: "Vaccine product name", kind: "text", required: true },
        { path: "vaccineManufacturer", label: "Manufacturer", kind: "text", required: true },
        { path: "batchLotNumber", label: "Batch / lot number", kind: "text", required: true },
        {
          path: "series",
          label: "Series",
          kind: "select",
          required: true,
          options: [
            { value: "primary", label: "Primary" },
            { value: "booster", label: "Booster" },
          ],
          help: "Primary: validFrom = vaccinationDate + 21 days.",
        },
      ],
    },
    {
      title: "Dates",
      fields: [
        {
          path: "vaccinationDate",
          label: "Vaccination date",
          kind: "date",
          required: true,
          help: "Dog must be ≥ 12 weeks old at vaccination.",
        },
        { path: "validFrom", label: "Valid from", kind: "date", required: true },
        { path: "validUntil", label: "Valid until", kind: "date", required: true },
        { path: "nextDueDate", label: "Next due date", kind: "date", required: true },
      ],
    },
    {
      title: "Authorization",
      fields: [
        { path: "authorizedVet", label: "Authorized vet", kind: "text", required: true },
      ],
    },
  ],
};

export const DOG_PROFILE: RecordTypeSchema = {
  recordType: "DOG_PROFILE",
  label: "Dog Profile",
  description: "Normalized identity credential (breed VBO, sex/neuter, DOB).",
  groups: [
    {
      title: "Identity",
      fields: [
        {
          path: "species",
          label: "Species",
          kind: "select",
          required: true,
          options: [
            { value: "dog", label: "Dog" },
            { value: "cat", label: "Cat" },
          ],
        },
        { path: "breedVbo", label: "Breed (VBO id)", kind: "text", required: true, placeholder: "VBO:0200798" },
        { path: "breedLabel", label: "Breed (label)", kind: "text", required: true },
        {
          path: "sex",
          label: "Sex",
          kind: "select",
          required: true,
          options: [
            { value: "male", label: "Male" },
            { value: "female", label: "Female" },
          ],
        },
        {
          path: "neuterStatus",
          label: "Neuter status",
          kind: "select",
          required: true,
          options: [
            { value: "intact", label: "Intact" },
            { value: "neutered", label: "Neutered" },
            { value: "spayed", label: "Spayed" },
          ],
        },
        { path: "dateOfBirth", label: "Date of birth", kind: "date", required: true },
      ],
    },
  ],
};

export const RECORD_TYPE_SCHEMAS: RecordTypeSchema[] = [RABIES_VACCINATION, DOG_PROFILE];

export function schemaFor(recordType: string): RecordTypeSchema | undefined {
  return RECORD_TYPE_SCHEMAS.find((s) => s.recordType === recordType);
}

/** Validate a single value against a field def; returns an error message or null. */
export function validateField(def: FieldDef, raw: string): string | null {
  const v = raw.trim();
  if (def.required && !v) return `${def.label} is required`;
  if (!v) return null;
  if (def.pattern && !def.pattern.test(v)) return def.patternMessage ?? `${def.label} is invalid`;
  return null;
}

/** Reassemble a flat dotted-path map into the nested `fields` object for the prepare request. */
export function buildFieldsObject(flat: Record<string, string>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [path, value] of Object.entries(flat)) {
    if (value === "") continue;
    const parts = path.split(".");
    let cursor = out;
    for (let i = 0; i < parts.length - 1; i++) {
      const key = parts[i]!;
      if (typeof cursor[key] !== "object" || cursor[key] === null) cursor[key] = {};
      cursor = cursor[key] as Record<string, unknown>;
    }
    cursor[parts[parts.length - 1]!] = value;
  }
  return out;
}

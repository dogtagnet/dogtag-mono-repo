// Browser shims for the Node globals circomlibjs (pulled in by @dogtag/standard's EdDSA-BabyJubjub
// consent signing) expects at runtime. Imported first in main.tsx, before any SDK code runs.
import { Buffer } from "buffer";

const g = globalThis as unknown as { Buffer?: typeof Buffer; global?: typeof globalThis };
if (!g.Buffer) g.Buffer = Buffer;
if (!g.global) g.global = globalThis;

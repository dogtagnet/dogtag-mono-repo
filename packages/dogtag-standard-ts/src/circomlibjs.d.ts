// Minimal ambient types for circomlibjs@0.1.7 (which ships no .d.ts). Only the surface the
// consent module uses: buildEddsa / buildBabyjub and the field-element helpers we touch.
declare module "circomlibjs" {
  /** A circomlibjs field-element wrapper (Montgomery-form Uint8Array internally). */
  export interface CircomField {
    e(x: string | number | bigint): unknown;
    toString(x: unknown): string;
  }

  export interface Eddsa {
    F: CircomField;
    prv2pub(prv: Uint8Array): [unknown, unknown];
    signPoseidon(prv: Uint8Array, msg: unknown): {R8: [unknown, unknown]; S: bigint};
    verifyPoseidon(msg: unknown, sig: {R8: [unknown, unknown]; S: bigint}, pub: [unknown, unknown]): boolean;
  }

  export interface Babyjub {
    F: CircomField;
  }

  export function buildEddsa(): Promise<Eddsa>;
  export function buildBabyjub(): Promise<Babyjub>;
}

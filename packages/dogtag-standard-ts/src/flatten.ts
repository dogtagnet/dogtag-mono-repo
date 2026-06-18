// Pinned flatten/keyPath grammar (impl §11.2 F2a — load-bearing, keyPath is hashed):
//   object key -> ".key"   (key NFC; reserved chars . [ ] rejected)
//   array elem -> "[i]"    (i base-10, no leading zeros)
//   root has no leading "."; an empty object/array -> a null-typed leaf at that path.
import {TypeTag, type TypedScalar} from "./types.js";
import {nfc} from "./encode.js";

const RESERVED = /[.[\]]/;

export interface FlatEntry {
  keyPath: string;
  scalar: TypedScalar;
}

function isTypedScalar(v: unknown): v is TypedScalar {
  return (
    typeof v === "object" &&
    v !== null &&
    "tag" in v &&
    "value" in v &&
    typeof (v as {tag: unknown}).tag === "number"
  );
}

/** Flatten a nested typed credential into pinned (keyPath, scalar) pairs. */
export function flatten(credential: unknown): FlatEntry[] {
  const out: FlatEntry[] = [];
  walk(credential, "", out);
  return out;
}

function walk(node: unknown, path: string, out: FlatEntry[]): void {
  if (isTypedScalar(node)) {
    out.push({keyPath: path, scalar: node});
    return;
  }
  if (Array.isArray(node)) {
    if (node.length === 0) {
      out.push({keyPath: path, scalar: {tag: TypeTag.Null, value: null}});
      return;
    }
    node.forEach((el, i) => walk(el, `${path}[${i}]`, out));
    return;
  }
  if (typeof node === "object" && node !== null) {
    const keys = Object.keys(node);
    if (keys.length === 0) {
      out.push({keyPath: path, scalar: {tag: TypeTag.Null, value: null}});
      return;
    }
    for (const k of keys) {
      const key = nfc(k);
      if (RESERVED.test(key)) throw new Error(`reserved char in object key: ${JSON.stringify(key)}`);
      const childPath = path === "" ? key : `${path}.${key}`;
      walk((node as Record<string, unknown>)[k], childPath, out);
    }
    return;
  }
  throw new Error(`non-typed leaf at ${JSON.stringify(path)} — wrap scalars as {tag,value}`);
}

type Token = {kind: "key"; key: string} | {kind: "index"; idx: number};

/** Tokenize a pinned keyPath into segments (reserved chars make this unambiguous). */
export function tokenizeKeyPath(keyPath: string): Token[] {
  const tokens: Token[] = [];
  let i = 0;
  let cur = "";
  const flushKey = () => {
    if (cur !== "") {
      tokens.push({kind: "key", key: cur});
      cur = "";
    }
  };
  while (i < keyPath.length) {
    const c = keyPath[i]!;
    if (c === ".") {
      flushKey();
      i++;
    } else if (c === "[") {
      flushKey();
      const end = keyPath.indexOf("]", i);
      if (end < 0) throw new Error("unterminated array index");
      const num = keyPath.slice(i + 1, end);
      if (!/^(0|[1-9][0-9]*)$/.test(num)) throw new Error(`bad array index: ${num}`);
      tokens.push({kind: "index", idx: Number(num)});
      i = end + 1;
    } else {
      cur += c;
      i++;
    }
  }
  flushKey();
  return tokens;
}

/** Rebuild a nested object/array of packed strings from flat (keyPath -> packed) pairs. */
export function unflatten(entries: Record<string, string>): unknown {
  const root: Record<string, unknown> = {};
  for (const [keyPath, packed] of Object.entries(entries)) {
    const tokens = tokenizeKeyPath(keyPath);
    let cursor: Record<string, unknown> | unknown[] = root;
    for (let t = 0; t < tokens.length; t++) {
      const tok = tokens[t]!;
      const last = t === tokens.length - 1;
      const nextTok = tokens[t + 1];
      if (tok.kind === "key") {
        const obj = cursor as Record<string, unknown>;
        if (last) {
          obj[tok.key] = packed;
        } else {
          if (obj[tok.key] === undefined) obj[tok.key] = nextTok!.kind === "index" ? [] : {};
          cursor = obj[tok.key] as Record<string, unknown> | unknown[];
        }
      } else {
        const arr = cursor as unknown[];
        if (last) {
          arr[tok.idx] = packed;
        } else {
          if (arr[tok.idx] === undefined) arr[tok.idx] = nextTok!.kind === "index" ? [] : {};
          cursor = arr[tok.idx] as Record<string, unknown> | unknown[];
        }
      }
    }
  }
  return root;
}

/**
 * The live mirror read seam.
 *
 * Deliberately thin: it maps HTTP outcomes onto {@link MirrorFetch} and does NOT verify. Verification
 * is `verifyContentAddress`, called by `resolveProviderProfile` over whatever this returns — keeping
 * them apart is what stops "the fetch succeeded" being mistaken for "the bytes are right", which is
 * the entire failure mode content addressing exists to remove.
 */

import { keccak256, toHex } from "viem";

import type { BytesDigestFn, ContentAddress } from "./contentAddress";
import type { FetchContentFn, MirrorFetch } from "./resolveProfile";

/** viem's keccak256 over raw bytes, as the verification seam wants it. */
export const keccakBytes: BytesDigestFn = (bytes) => keccak256(bytes);

/**
 * The one digest a publication uses, over TEXT or BYTES.
 *
 * Both spellings must reach the same function, because a publication computes two addresses - the
 * logo's and the blob's - and the blob's digest COVERS the logo's address. Two functions here would
 * let those be computed differently, so the anchored blob would commit to a logo address no consumer
 * could reproduce, and the logo would read as altered on a perfectly honest publication.
 *
 * A string is hashed as its UTF-8 bytes (`toHex`), which is exactly what the mirror hashes when the
 * same blob arrives as a request body.
 */
export const publicationDigest = (content: string | Uint8Array): `0x${string}` =>
  typeof content === "string" ? keccak256(toHex(content)) : keccak256(content);

/**
 * Build a reader against one mirror base (e.g. the indexer at `:46001`).
 *
 * The base is a deployment setting and never provider-supplied. That is not a trust claim about the
 * base — a content address makes the host irrelevant, which is the point — but a provider-named host
 * would be a request the provider controls the target of, and there is no reason to offer that.
 */
/**
 * Publish bytes to the mirror under their own content address.
 *
 * THROWS on any non-2xx. A publication step that failed must stop the sequence rather than resolve
 * quietly: the caller sends the anchor transaction next, and an anchor naming content the mirror
 * does not hold is indistinguishable, to every consumer, from a provider who published nothing.
 *
 * The mirror recomputes the address over the body and refuses a mismatch. That refusal is not this
 * function's to anticipate - it is reported as the failure it is.
 */
export async function putMirrorContent(
  baseUrl: string,
  address: ContentAddress,
  bytes: Uint8Array,
  mediaType: string,
  token?: string,
): Promise<void> {
  const base = baseUrl.replace(/\/+$/, "");
  const headers: Record<string, string> = { "content-type": mediaType };
  if (token) headers.authorization = `Bearer ${token}`;
  let response: Response;
  try {
    response = await fetch(`${base}/v1/content/${address}`, {
      method: "PUT",
      headers,
      body: bytes as BodyInit,
    });
  } catch (error) {
    throw new Error(
      `the content mirror could not be reached: ${
        error instanceof Error && error.message ? error.message : "network error"
      }`,
    );
  }
  if (!response.ok) {
    let detail = `HTTP ${response.status}`;
    try {
      const body = (await response.json()) as { error?: unknown };
      if (typeof body?.error === "string") detail = body.error;
    } catch {
      // Leave the status as the detail; a body we could not read is not a better message.
    }
    throw new Error(`the content mirror refused this publication: ${detail}`);
  }
}

export function mirrorContentReader(baseUrl: string): FetchContentFn {
  const base = baseUrl.replace(/\/+$/, "");
  return async (address: ContentAddress): Promise<MirrorFetch> => {
    let response: Response;
    try {
      response = await fetch(`${base}/v1/content/${address}`, {
        headers: { accept: "*/*" },
      });
    } catch (error) {
      return {
        state: "unavailable",
        reason: error instanceof Error && error.message ? error.message : "network error",
      };
    }
    if (response.status === 404) return { state: "absent" };
    if (!response.ok) {
      return { state: "unavailable", reason: `mirror responded ${response.status}` };
    }
    let bytes: Uint8Array;
    try {
      bytes = new Uint8Array(await response.arrayBuffer());
    } catch (error) {
      return {
        state: "unavailable",
        reason: error instanceof Error && error.message ? error.message : "body could not be read",
      };
    }
    return {
      state: "found",
      bytes,
      mediaType: response.headers.get("content-type") ?? "",
    };
  };
}

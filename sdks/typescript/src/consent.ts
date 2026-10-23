/**
 * Owner consent tokens: check that a person approved exactly this request,
 * or confirmed exactly this reply.
 *
 * A consent token is an EdDSA JWT (`typ: "chakramcp-consent+jwt"`) signed
 * with the relay's key, published at `{relay}/.well-known/jwks.json`. Its
 * `sha256` is the SHA-256 of the RFC 8785 (JCS) canonical JSON of what was
 * delivered: the input (`kind: "request"`) or the released reply
 * `{status, output?, error?}` (`kind: "reply"`). Uses WebCrypto (Node 20+,
 * modern browsers); no dependencies.
 */

export interface ConsentClaims {
  /** The relay's URL. */
  iss: string;
  /** The consent request id. */
  jti: string;
  kind: "request" | "reply";
  /** Invocation id. */
  inv: string;
  /** Capability id. */
  cap: string;
  /** Target agent id. */
  granter: string;
  /** Requesting agent id. */
  grantee: string;
  /** The approving user's id. */
  approver: string;
  /** The standing consent that covered the request, if one did. */
  consent?: string;
  sha256: string;
  iat: number;
  exp: number;
}

export type ConsentTokenErrorCode =
  | "malformed"
  | "wrong_type"
  | "unknown_key"
  | "bad_signature"
  | "expired"
  | "value_mismatch";

export class ConsentTokenError extends Error {
  constructor(
    public readonly code: ConsentTokenErrorCode,
    message: string,
  ) {
    super(message);
    this.name = "ConsentTokenError";
  }
}

/** The JWT `typ` of a consent token. */
export const CONSENT_TOKEN_TYPE = "chakramcp-consent+jwt";

/**
 * RFC 8785 (JCS) canonical JSON: object keys sorted by UTF-16 code units,
 * no whitespace, ECMAScript number and string serialization.
 */
export function canonicalJson(value: unknown): string {
  if (value === null || typeof value !== "object") {
    if (typeof value === "number" && !Number.isFinite(value)) {
      throw new TypeError("JCS can't encode a non-finite number");
    }
    // JSON.stringify serializes numbers and strings exactly as JCS requires.
    const s = JSON.stringify(value);
    if (s === undefined) throw new TypeError(`JCS can't encode ${typeof value}`);
    return s;
  }
  if (Array.isArray(value)) {
    return `[${value.map((v) => canonicalJson(v)).join(",")}]`;
  }
  const obj = value as Record<string, unknown>;
  // Default sort compares UTF-16 code units, as JCS requires.
  const keys = Object.keys(obj)
    .filter((k) => obj[k] !== undefined)
    .sort();
  return `{${keys.map((k) => `${JSON.stringify(k)}:${canonicalJson(obj[k])}`).join(",")}}`;
}

/** Hex SHA-256 of the canonical JSON of `value`. */
export async function sha256Jcs(value: unknown): Promise<string> {
  const bytes = new TextEncoder().encode(canonicalJson(value));
  const digest = await globalThis.crypto.subtle.digest("SHA-256", bytes);
  return Array.from(new Uint8Array(digest), (b) => b.toString(16).padStart(2, "0")).join("");
}

interface Jwk {
  kid?: string;
  kty?: string;
  crv?: string;
  x?: string;
  [k: string]: unknown;
}

export interface VerifyConsentTokenOptions {
  /** Where the relay publishes its keys, e.g. `https://relay.chakramcp.com/.well-known/jwks.json`. */
  jwksUrl?: string;
  /** The keys themselves, instead of `jwksUrl`. */
  jwks?: { keys: Jwk[] };
  /**
   * What you were delivered: the input (`request`) or the reply
   * `{status, output?, error?}` (`reply`). When given, its hash must match.
   */
  value?: unknown;
  /** The time to check expiry against (tests). */
  now?: Date;
  /** `fetch` to use for `jwksUrl` (tests). */
  fetch?: typeof fetch;
}

function b64urlDecode(s: string): Uint8Array<ArrayBuffer> {
  const b64 = s.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(s.length / 4) * 4, "=");
  const bin = atob(b64);
  const out = new Uint8Array(new ArrayBuffer(bin.length));
  for (let i = 0; i < bin.length; i++) out[i] = bin.charCodeAt(i);
  return out;
}

function decodeJson<T>(part: string): T {
  try {
    return JSON.parse(new TextDecoder().decode(b64urlDecode(part))) as T;
  } catch {
    throw new ConsentTokenError("malformed", "the token isn't a JWT");
  }
}

/**
 * Check a consent token: its type, its signature against the relay's keys,
 * its expiry and, given `value`, that it covers exactly that value. Returns
 * the claims; throws `ConsentTokenError` otherwise.
 */
export async function verifyConsentToken(
  token: string,
  opts: VerifyConsentTokenOptions,
): Promise<ConsentClaims> {
  const [headerB64, payloadB64, sigB64, extra] = token.split(".");
  if (!headerB64 || !payloadB64 || !sigB64 || extra !== undefined) {
    throw new ConsentTokenError("malformed", "the token isn't a JWT");
  }
  const header = decodeJson<{ alg?: string; kid?: string; typ?: string }>(headerB64);
  if (header.typ !== CONSENT_TOKEN_TYPE || header.alg !== "EdDSA") {
    throw new ConsentTokenError("wrong_type", "not a ChakraMCP consent token");
  }

  let jwks = opts.jwks;
  if (!jwks) {
    if (!opts.jwksUrl) throw new TypeError("pass jwksUrl or jwks");
    const res = await (opts.fetch ?? fetch)(opts.jwksUrl);
    if (!res.ok) throw new Error(`couldn't fetch ${opts.jwksUrl}: ${res.status}`);
    jwks = (await res.json()) as { keys: Jwk[] };
  }
  const jwk = jwks.keys.find((k) => k.kid === header.kid);
  if (!jwk || jwk.kty !== "OKP" || jwk.crv !== "Ed25519" || !jwk.x) {
    throw new ConsentTokenError("unknown_key", "signed with a key the relay doesn't publish");
  }
  const key = await globalThis.crypto.subtle.importKey(
    "jwk",
    { kty: "OKP", crv: "Ed25519", x: jwk.x },
    { name: "Ed25519" },
    false,
    ["verify"],
  );
  const signed = new TextEncoder().encode(`${headerB64}.${payloadB64}`);
  const ok = await globalThis.crypto.subtle.verify(
    { name: "Ed25519" },
    key,
    b64urlDecode(sigB64),
    signed,
  );
  if (!ok) throw new ConsentTokenError("bad_signature", "the signature doesn't match");

  const claims = decodeJson<ConsentClaims>(payloadB64);
  const now = Math.floor((opts.now ?? new Date()).getTime() / 1000);
  if (claims.exp <= now) throw new ConsentTokenError("expired", "the token has expired");
  if (opts.value !== undefined && (await sha256Jcs(opts.value)) !== claims.sha256) {
    throw new ConsentTokenError("value_mismatch", "the token doesn't cover this value");
  }
  return claims;
}

import { describe, expect, it } from "vitest";
// The relay's RFC 8785 vectors (backend/relay/tests/data/jcs/vectors.json):
// every implementation must produce the same bytes.
import vectors from "./fixtures/jcs-vectors.json";
import {
  canonicalJson,
  ChakraMCP,
  ConsentTokenError,
  sha256Jcs,
  verifyConsentToken,
  type ConsentClaims,
} from "../src/index.js";

describe("canonicalJson", () => {
  it("matches the RFC 8785 vectors the relay uses", () => {
    expect(vectors.length).toBeGreaterThan(0);
    for (const [input, canonical] of vectors as [string, string][]) {
      expect(canonicalJson(JSON.parse(input))).toBe(canonical);
    }
  });

  it("ignores key order and whitespace", async () => {
    const a = JSON.parse('{"b": 1, "a": [1, 2, {"d": "x", "c": null}]}');
    const b = JSON.parse('{"a":[1,2,{"c":null,"d":"x"}],"b":1}');
    expect(await sha256Jcs(a)).toBe(await sha256Jcs(b));
  });
});

function b64url(bytes: Uint8Array | string): string {
  const raw = typeof bytes === "string" ? new TextEncoder().encode(bytes) : bytes;
  let bin = "";
  for (const b of raw) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

async function signedToken(
  claims: Partial<ConsentClaims>,
  typ = "chakramcp-consent+jwt",
): Promise<{ token: string; jwks: { keys: Record<string, unknown>[] } }> {
  const pair = (await crypto.subtle.generateKey({ name: "Ed25519" }, true, [
    "sign",
    "verify",
  ])) as CryptoKeyPair;
  const jwk = await crypto.subtle.exportKey("jwk", pair.publicKey);
  const header = b64url(JSON.stringify({ alg: "EdDSA", kid: "k1", typ }));
  const payload = b64url(JSON.stringify(claims));
  const sig = new Uint8Array(
    await crypto.subtle.sign({ name: "Ed25519" }, pair.privateKey, new TextEncoder().encode(`${header}.${payload}`)),
  );
  return { token: `${header}.${payload}.${b64url(sig)}`, jwks: { keys: [{ ...jwk, kid: "k1" }] } };
}

describe("verifyConsentToken", () => {
  const input = { date: "2026-10-28", minutes: 45 };
  const now = Math.floor(Date.now() / 1000);

  it("accepts a token for exactly this value", async () => {
    const { token, jwks } = await signedToken({
      kind: "request",
      jti: "cr-1",
      sha256: await sha256Jcs(input),
      iat: now,
      exp: now + 3600,
    });
    const claims = await verifyConsentToken(token, { jwks, value: { minutes: 45, date: "2026-10-28" } });
    expect(claims.jti).toBe("cr-1");
    await expect(verifyConsentToken(token, { jwks, value: { ...input, minutes: 60 } })).rejects.toMatchObject({
      code: "value_mismatch",
    });
  });

  it("refuses other token types, expired tokens and unknown keys", async () => {
    const claims = { sha256: await sha256Jcs(input), iat: now, exp: now + 60 };
    const bearer = await signedToken(claims, "JWT");
    await expect(verifyConsentToken(bearer.token, { jwks: bearer.jwks })).rejects.toBeInstanceOf(ConsentTokenError);
    const expired = await signedToken({ ...claims, exp: now - 1 });
    await expect(verifyConsentToken(expired.token, { jwks: expired.jwks })).rejects.toMatchObject({ code: "expired" });
    const other = await signedToken(claims);
    await expect(verifyConsentToken(other.token, { jwks: expired.jwks })).rejects.toMatchObject({
      code: "bad_signature",
    });
  });

  it("fetches the relay's keys", async () => {
    const { token, jwks } = await signedToken({ sha256: "x", iat: now, exp: now + 60 });
    const fetchStub = (async () => new Response(JSON.stringify(jwks))) as unknown as typeof fetch;
    const claims = await verifyConsentToken(token, { jwksUrl: "https://relay/.well-known/jwks.json", fetch: fetchStub });
    expect(claims.sha256).toBe("x");
  });
});

describe("inbox.notices", () => {
  it("claims the agent's notices", async () => {
    const seen: string[] = [];
    const fetchStub = (async (input: RequestInfo | URL) => {
      seen.push(String(input instanceof Request ? input.url : input));
      return new Response(JSON.stringify([{ id: "n1", kind: "consent_requested", created_at: "x" }]));
    }) as typeof fetch;
    const sdk = new ChakraMCP({ apiKey: "ck_test", relayUrl: "http://relay", fetch: fetchStub });
    const notices = await sdk.inbox.notices("agent-1", { limit: 5 });
    expect(notices[0]?.kind).toBe("consent_requested");
    expect(seen[0]).toContain("/v1/inbox/notices?agent_id=agent-1&limit=5");
  });
});

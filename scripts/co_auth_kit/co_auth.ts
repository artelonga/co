/**
 * CO-166: co_auth.ts — CO session verification kit (Deno/Node.js compatible)
 *
 * Verifies CO session JWTs using the public key from the CO server's JWKS
 * endpoint. No shared secret needed — uses ES256 (asymmetric signing).
 *
 * Usage:
 *   import { verifyCoSession, getCoSessionCookie } from "./co_auth.ts";
 *
 *   const token = getCoSessionCookie(req.headers.get("cookie") ?? "");
 *   if (!token) return Response.json({ error: "not authenticated" }, { status: 401 });
 *
 *   const user = await verifyCoSession(token, "https://co.artelonga.com.br");
 *   if (!user) return Response.json({ error: "invalid token" }, { status: 401 });
 *
 *   // user.sub  — CO user ID
 *   // user.email — CO user email
 *   // user.tier  — "player" | "admin" | "anon" | ...
 */

export interface JwkKey {
  kty: string;
  crv: string;
  x: string;
  y: string;
  kid: string;
  alg?: string;
  use?: string;
}

export interface JwksResponse {
  keys: JwkKey[];
}

export interface CoUser {
  sub: string;
  email: string;
  tier: string;
  exp: number;
  iat: number;
}

/**
 * Fetch the JWKS from the CO server and cache in memory.
 * Cache TTL: 5 minutes (JWKS rarely changes; key rotations are graceful).
 */
const jwksCache = new Map<string, { keys: JwkKey[]; fetchedAt: number }>();
const JWKS_CACHE_TTL_MS = 5 * 60 * 1000;

async function fetchJwks(coServerUrl: string): Promise<JwkKey[]> {
  const cached = jwksCache.get(coServerUrl);
  if (cached && Date.now() - cached.fetchedAt < JWKS_CACHE_TTL_MS) {
    return cached.keys;
  }

  const resp = await fetch(`${coServerUrl}/.well-known/jwks.json`);
  if (!resp.ok) {
    throw new Error(`Failed to fetch JWKS from ${coServerUrl}: ${resp.status}`);
  }
  const jwks: JwksResponse = await resp.json();
  jwksCache.set(coServerUrl, { keys: jwks.keys, fetchedAt: Date.now() });
  return jwks.keys;
}

/**
 * Decode the header of a JWT without verifying (to extract kid).
 */
function decodeJwtHeader(token: string): { kid?: string; alg?: string } {
  const parts = token.split(".");
  if (parts.length !== 3) return {};
  try {
    const header = JSON.parse(atob(parts[0].replace(/-/g, "+").replace(/_/g, "/")));
    return header;
  } catch {
    return {};
  }
}

/**
 * Decode the payload of a JWT without verifying (for fallback inspection).
 */
function decodeJwtPayload(token: string): Record<string, unknown> | null {
  const parts = token.split(".");
  if (parts.length !== 3) return null;
  try {
    return JSON.parse(atob(parts[1].replace(/-/g, "+").replace(/_/g, "/")));
  } catch {
    return null;
  }
}

/**
 * Import a JWK (EC P-256) as a CryptoKey for verification.
 */
async function importJwk(jwk: JwkKey): Promise<CryptoKey> {
  return await crypto.subtle.importKey(
    "jwk",
    {
      kty: jwk.kty,
      crv: jwk.crv,
      x: jwk.x,
      y: jwk.y,
      ext: true,
    },
    { name: "ECDSA", namedCurve: "P-256" },
    false,
    ["verify"],
  );
}

/**
 * base64url decode a string to a Uint8Array.
 */
function base64urlDecode(input: string): Uint8Array {
  const base64 = input.replace(/-/g, "+").replace(/_/g, "/");
  const padded = base64.padEnd(base64.length + ((4 - (base64.length % 4)) % 4), "=");
  const binary = atob(padded);
  return new Uint8Array(binary.length).map((_, i) => binary.charCodeAt(i));
}

/**
 * Verify a CO session JWT using JWKS (ES256). Returns the claims on success or null on failure.
 *
 * @param token      Raw JWT string (not "Bearer <token>")
 * @param coServerUrl  Base URL of the CO server, e.g. "https://co.artelonga.com.br"
 */
export async function verifyCoSession(
  token: string,
  coServerUrl: string,
): Promise<CoUser | null> {
  try {
    const header = decodeJwtHeader(token);
    const keys = await fetchJwks(coServerUrl);

    // Find matching key by kid, or try all keys if no kid.
    const candidates = header.kid ? keys.filter((k) => k.kid === header.kid) : keys;
    if (candidates.length === 0) {
      return null;
    }

    const parts = token.split(".");
    if (parts.length !== 3) return null;

    const signingInput = `${parts[0]}.${parts[1]}`;
    const signature = base64urlDecode(parts[2]);
    const data = new TextEncoder().encode(signingInput);

    for (const jwk of candidates) {
      try {
        const key = await importJwk(jwk);
        const valid = await crypto.subtle.verify(
          { name: "ECDSA", hash: "SHA-256" },
          key,
          signature,
          data,
        );
        if (!valid) continue;

        // Signature valid — decode and check expiry.
        const payload = decodeJwtPayload(token);
        if (!payload) return null;

        const now = Math.floor(Date.now() / 1000);
        if (typeof payload.exp === "number" && payload.exp < now) {
          return null; // expired
        }

        return {
          sub: String(payload.sub ?? ""),
          email: String(payload.email ?? ""),
          tier: String(payload.tier ?? ""),
          exp: Number(payload.exp ?? 0),
          iat: Number(payload.iat ?? 0),
        };
      } catch {
        continue;
      }
    }

    return null;
  } catch {
    return null;
  }
}

/**
 * Extract the CO session token from a `Cookie` header value.
 *
 * @param cookieHeader  Value of the `Cookie` HTTP header
 */
export function getCoSessionCookie(cookieHeader: string): string | null {
  for (const part of cookieHeader.split(";")) {
    const trimmed = part.trim();
    if (trimmed.startsWith("session=")) {
      const value = trimmed.slice("session=".length).trim();
      return value.length > 0 ? value : null;
    }
  }
  return null;
}

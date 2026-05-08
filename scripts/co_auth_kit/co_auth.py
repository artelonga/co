"""
CO-166: co_auth.py — CO session verification kit (Python)

Verifies CO session JWTs using the public key from the CO server's JWKS
endpoint. No shared secret needed — uses ES256 (asymmetric signing).

Requires:
    pip install PyJWT cryptography

Usage:
    from co_auth import verify_co_session, get_co_session_cookie

    cookie_header = request.headers.get("Cookie", "")
    token = get_co_session_cookie(cookie_header)
    if not token:
        return {"error": "not authenticated"}, 401

    user = verify_co_session(token, "https://co.artelonga.com.br")
    if not user:
        return {"error": "invalid token"}, 401

    # user["sub"]   — CO user ID
    # user["email"] — CO user email
    # user["tier"]  — "player" | "admin" | "anon" | ...
"""

import base64
import json
import urllib.request
from datetime import datetime, timezone
from functools import lru_cache
from typing import Optional

try:
    import jwt
    from jwt.algorithms import ECAlgorithm
except ImportError as e:
    raise ImportError(
        "PyJWT with cryptography is required: pip install PyJWT cryptography"
    ) from e


# ---------------------------------------------------------------------------
# JWKS fetching (simple in-process cache)
# ---------------------------------------------------------------------------

_jwks_cache: dict[str, tuple[list[dict], float]] = {}
_JWKS_CACHE_TTL = 300  # 5 minutes


def _fetch_jwks(co_server_url: str) -> list[dict]:
    """Fetch and cache the JWKS from the CO server."""
    import time

    cached = _jwks_cache.get(co_server_url)
    if cached is not None:
        keys, fetched_at = cached
        if time.time() - fetched_at < _JWKS_CACHE_TTL:
            return keys

    url = f"{co_server_url.rstrip('/')}/.well-known/jwks.json"
    with urllib.request.urlopen(url, timeout=5) as resp:
        data = json.loads(resp.read())

    keys = data.get("keys", [])
    _jwks_cache[co_server_url] = (keys, time.time())
    return keys


# ---------------------------------------------------------------------------
# JWT verification
# ---------------------------------------------------------------------------


def verify_co_session(token: str, co_server_url: str) -> Optional[dict]:
    """
    Verify a CO session JWT using JWKS (ES256). No shared secret needed.

    Args:
        token:          Raw JWT string (not "Bearer <token>")
        co_server_url:  Base URL of the CO server

    Returns:
        dict with {sub, email, tier, exp, iat} on success, None on failure.
    """
    try:
        # Decode header to find kid.
        header = jwt.get_unverified_header(token)
        kid = header.get("kid")
        alg = header.get("alg", "ES256")

        if alg != "ES256":
            return None

        keys = _fetch_jwks(co_server_url)

        # Find matching key by kid (or try all if no kid).
        candidates = [k for k in keys if k.get("kid") == kid] if kid else keys
        if not candidates:
            return None

        for jwk_data in candidates:
            try:
                # Convert JWK to a public key usable by PyJWT.
                public_key = ECAlgorithm.from_jwk(json.dumps(jwk_data))

                payload = jwt.decode(
                    token,
                    public_key,
                    algorithms=["ES256"],
                    options={"verify_exp": True},
                )

                return {
                    "sub": payload.get("sub", ""),
                    "email": payload.get("email", ""),
                    "tier": payload.get("tier", ""),
                    "exp": payload.get("exp", 0),
                    "iat": payload.get("iat", 0),
                }
            except Exception:
                continue

        return None

    except Exception:
        return None


# ---------------------------------------------------------------------------
# Cookie extraction
# ---------------------------------------------------------------------------


def get_co_session_cookie(cookie_header: str) -> Optional[str]:
    """
    Extract the CO session token from a Cookie header value.

    Args:
        cookie_header: Value of the `Cookie` HTTP header

    Returns:
        The raw JWT string, or None if not found.
    """
    for part in cookie_header.split(";"):
        part = part.strip()
        if part.startswith("session="):
            value = part[len("session=") :].strip()
            return value if value else None
    return None


# ---------------------------------------------------------------------------
# Example / quick self-test
# ---------------------------------------------------------------------------

if __name__ == "__main__":
    import sys

    if len(sys.argv) < 3:
        print("Usage: python co_auth.py <co_server_url> <token>")
        sys.exit(1)

    co_url = sys.argv[1]
    tok = sys.argv[2]

    result = verify_co_session(tok, co_url)
    if result:
        print("Valid token:")
        print(json.dumps(result, indent=2))
    else:
        print("Invalid or expired token")
        sys.exit(1)

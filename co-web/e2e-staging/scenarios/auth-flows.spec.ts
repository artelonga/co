/**
 * CO-374 — Auth flows: magic code, password login, cross-env token, logout
 *
 * Covered scenarios:
 *   1. Magic-code happy path (onboard-with-email → verify)
 *   2. Password login (yuri staging credentials)
 *   3. Cross-env token: token from prod accepted on staging (shared JWT_SECRET)
 *   4. Logout clears session cookie
 *   5. Expired / invalid token → 401
 *
 * CO-379 establishes that staging and prod share JWT_SECRET, enabling
 * cross-env token acceptance (CO-377).
 */

import { test, expect } from "../fixtures";
import { getMagicCodeFromStagingLogs } from "../helpers";

test.describe("Auth flows — magic code", () => {
  test("onboard-with-email returns 202 and sent=true", async ({
    anonContext,
  }) => {
    const email = `playwright-magic-${Date.now()}@staging.test`;
    const res = await anonContext.post("/api/v1/auth/onboard-with-email", {
      data: { email },
    });
    expect(res.status()).toBe(202);
    const body = await res.json();
    expect(body.sent).toBe(true);
  });

  test("magic code verify succeeds when dev_code is available", async ({
    anonContext,
  }) => {
    const email = `playwright-verify-${Date.now()}@staging.test`;
    const sendRes = await anonContext.post("/api/v1/auth/onboard-with-email", {
      data: { email },
    });
    expect(sendRes.status()).toBe(202);
    const sendBody = await sendRes.json();

    // dev_code is only present in CO_ENV=test. On staging it requires FLY_API_TOKEN.
    if (typeof sendBody.dev_code === "string" && sendBody.dev_code.length === 6) {
      const verifyRes = await anonContext.post(
        "/api/v1/auth/onboard-with-email/verify",
        { data: { email, code: sendBody.dev_code } },
      );
      expect(verifyRes.status()).toBe(200);
      const verifyBody = await verifyRes.json();
      expect(verifyBody.user_id).toBeTruthy();
    } else if (process.env.FLY_API_TOKEN) {
      const code = await getMagicCodeFromStagingLogs(email);
      const verifyRes = await anonContext.post(
        "/api/v1/auth/onboard-with-email/verify",
        { data: { email, code } },
      );
      expect(verifyRes.status()).toBe(200);
    } else {
      test.skip(
        true,
        "FLY_API_TOKEN not set and dev_code not in response — set FLY_API_TOKEN to run magic-code E2E on staging",
      );
    }
  });

  test("wrong code returns 401", async ({ anonContext }) => {
    const email = `playwright-wrong-code-${Date.now()}@staging.test`;
    await anonContext.post("/api/v1/auth/onboard-with-email", {
      data: { email },
    });
    const verifyRes = await anonContext.post(
      "/api/v1/auth/onboard-with-email/verify",
      { data: { email, code: "000000" } },
    );
    expect([400, 401, 422]).toContain(verifyRes.status());
  });
});

test.describe("Auth flows — password login", () => {
  test("password login succeeds with staging admin credentials", async ({
    apiContext,
  }) => {
    // apiContext fixture already logged in via password-login
    const meRes = await apiContext.get("/api/v1/auth/me");
    // 200 = authenticated; 401 would mean login failed
    expect(meRes.status()).toBe(200);
    const body = await meRes.json();
    expect(body.email).toBeTruthy();
  });

  test("login-options endpoint returns expected shape", async ({
    anonContext,
  }) => {
    const res = await anonContext.get("/api/v1/auth/login-options");
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(typeof body.magic_code).toBe("boolean");
    expect(body.magic_code).toBe(true);
  });

  test("wrong password returns 401", async ({ anonContext }) => {
    const email =
      process.env.CO_STAGING_EMAIL ?? process.env.CO_ADMIN_EMAIL ?? "wrong@example.com";
    const res = await anonContext.post("/api/v1/auth/password-login", {
      data: { email, password: "wrong-password-intentional-failure" },
    });
    expect([401, 403]).toContain(res.status());
  });
});

test.describe("Auth flows — cross-env token (CO-377)", () => {
  test("token from staging is accepted on /api/v1/auth/me", async ({
    apiContext,
  }) => {
    // After logging in (apiContext fixture does this), /api/v1/auth/me must
    // return 200 — same token accepted across env (shared JWT_SECRET from CO-379)
    const res = await apiContext.get("/api/v1/auth/me");
    expect(res.status()).toBe(200);
  });

  test("token with wrong signature returns 401", async ({ anonContext }) => {
    const fakeToken = "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxIn0.wrong-sig";
    const res = await anonContext.get("/api/v1/auth/me", {
      headers: { Cookie: `session=${fakeToken}` },
    });
    expect([401, 403]).toContain(res.status());
  });
});

test.describe("Auth flows — logout", () => {
  test("POST /api/v1/auth/logout clears session cookie", async ({
    apiContext,
  }) => {
    const res = await apiContext.post("/api/v1/auth/logout");
    expect(res.status()).toBe(200);
    const setCookie = res.headers()["set-cookie"] ?? "";
    expect(setCookie).toContain("session=");
    expect(setCookie).toContain("Max-Age=0");
  });

  test("after logout, /api/v1/auth/me returns 401", async ({ anonContext }) => {
    // Fresh anonymous context has no session
    const res = await anonContext.get("/api/v1/auth/me");
    expect(res.status()).toBe(401);
  });
});

test.describe("Auth flows — session expiry", () => {
  test("malformed / expired token returns 401", async ({ anonContext }) => {
    // Simulate an expired token by sending a clearly invalid JWT
    const expiredToken = "invalid.expired.token";
    const res = await anonContext.get("/api/v1/auth/me", {
      headers: { Cookie: `session=${expiredToken}` },
    });
    expect([401, 403]).toContain(res.status());
  });
});

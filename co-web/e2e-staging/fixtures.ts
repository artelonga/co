import { test as base, expect, type APIRequestContext } from "@playwright/test";

export interface StagingFixtures {
  /** Authenticated API context for the staging admin user (yuri) */
  apiContext: APIRequestContext;
  /** Unauthenticated API context */
  anonContext: APIRequestContext;
}

export const test = base.extend<StagingFixtures>({
  apiContext: async ({ playwright }, use) => {
    const baseURL =
      process.env.STAGING_URL ?? "https://staging.co.artelonga.com.br";

    // CO-401: prefer the capability-scoped CO_STAGING_ADMIN_TOKEN (least-
    // privilege, CO-448). It is seeded on staging boot and carries exactly the
    // capabilities this suite exercises, so CI never needs the admin password.
    // Fall back to password-login only when the token is absent (local runs).
    const token = process.env.CO_STAGING_ADMIN_TOKEN ?? "";
    if (token) {
      const ctx = await playwright.request.newContext({
        baseURL,
        extraHTTPHeaders: { Authorization: `Bearer ${token}` },
      });
      await use(ctx);
      await ctx.dispose();
      return;
    }

    const ctx = await playwright.request.newContext({ baseURL });
    const email =
      process.env.CO_STAGING_EMAIL ?? process.env.CO_ADMIN_EMAIL ?? "";
    const password =
      process.env.CO_STAGING_PASSWORD ?? process.env.CO_ADMIN_PASSWORD ?? "";
    if (!email || !password) {
      throw new Error(
        "Set CO_STAGING_ADMIN_TOKEN (preferred) or CO_STAGING_EMAIL + CO_STAGING_PASSWORD for staging tests.",
      );
    }
    const res = await ctx.post("/api/v1/auth/password-login", {
      data: { email, password },
    });
    if (!res.ok()) {
      throw new Error(
        `Staging login failed (${res.status()}) — check CO_STAGING_EMAIL / CO_STAGING_PASSWORD.`,
      );
    }
    await use(ctx);
    await ctx.dispose();
  },

  anonContext: async ({ playwright }, use) => {
    const baseURL =
      process.env.STAGING_URL ?? "https://staging.co.artelonga.com.br";
    const ctx = await playwright.request.newContext({ baseURL });
    await use(ctx);
    await ctx.dispose();
  },
});

export { expect };

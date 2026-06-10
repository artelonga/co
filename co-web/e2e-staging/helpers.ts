import { type Page } from "@playwright/test";
import { execSync } from "node:child_process";

export const STAGING_APP = "co-artelonga-staging";
export const STAGING_URL =
  process.env.STAGING_URL ?? "https://staging.co.artelonga.com.br";

/** Login the page context as the staging admin (yuri) via password-login */
export async function loginAsYuri(page: Page): Promise<void> {
  const email =
    process.env.CO_STAGING_EMAIL ?? process.env.CO_ADMIN_EMAIL ?? "";
  const password =
    process.env.CO_STAGING_PASSWORD ?? process.env.CO_ADMIN_PASSWORD ?? "";
  await page.request.post(`${STAGING_URL}/api/v1/auth/password-login`, {
    data: { email, password },
  });
}

/**
 * Wait for a magic code to appear in Fly staging logs for the given email.
 * Requires FLY_API_TOKEN in the environment.
 * CO_EMAIL_PROVIDER=null on staging: codes go to server logs, never to SMTP.
 */
export async function getMagicCodeFromStagingLogs(
  email: string,
  timeoutMs = 30_000,
): Promise<string> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const logs = execSync(`flyctl logs -a ${STAGING_APP} --no-tail 2>&1`, {
        env: { ...process.env },
        timeout: 10_000,
      }).toString();
      const match = logs.match(
        new RegExp(
          `[Vv]erification code.*?${email.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}.*?:\\s*(\\d{6})`,
        ),
      );
      if (match) return match[1];
    } catch {
      // flyctl not available yet — keep retrying
    }
    await new Promise((r) => setTimeout(r, 2_000));
  }
  throw new Error(
    `Timed out (${timeoutMs}ms) waiting for magic code for ${email} in staging logs`,
  );
}

/**
 * Resync staging to the fixture baseline.
 * Only runs when STAGING_RESET=1 is set — opt-in to avoid accidental wipes.
 */
export async function resyncStagingBaseline(): Promise<void> {
  if (process.env.STAGING_RESET !== "1") return;
  execSync(
    `flyctl ssh console -a ${STAGING_APP} -C "touch /data/uat-reset.flag"`,
    { env: { ...process.env }, timeout: 15_000 },
  );
  execSync(`flyctl machine restart -a ${STAGING_APP}`, {
    env: { ...process.env },
    timeout: 30_000,
  });
  await waitForHealth(90_000);
}

async function waitForHealth(timeoutMs: number): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    try {
      const res = await fetch(`${STAGING_URL}/api/health`);
      if (res.ok) {
        const body = (await res.json()) as { status?: string };
        if (body.status === "ok") return;
      }
    } catch {
      // Not ready yet
    }
    await new Promise((r) => setTimeout(r, 2_000));
  }
  throw new Error(`Staging health check timed out after ${timeoutMs}ms`);
}

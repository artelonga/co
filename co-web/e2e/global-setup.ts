import { execSync, spawn } from "child_process";
import { join } from "path";

const SERVER_PORT = 3000;
const HEALTH_URL = `http://localhost:${SERVER_PORT}/api/health`;
const MAX_WAIT_MS = 60_000;
const POLL_INTERVAL_MS = 500;

/** Create the shared e2e-test universe owned by yuri (idempotent — 409 is OK).
 *  Board fixture tests create projects inside this universe so they appear in
 *  the authenticated sidebar without a per-universe-key universe per test. */
async function ensureTestUniverse(base: string): Promise<void> {
  try {
    const loginRes = await fetch(`${base}/api/v1/auth/uat-login`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ email: "yuri@uat.local", password: "uat" }),
    });
    if (!loginRes.ok) return; // uat-login not enabled (non-test env)
    const cookie = loginRes.headers.get("set-cookie")?.split(";")[0] ?? "";
    await fetch(`${base}/api/v1/universes`, {
      method: "POST",
      headers: { "Content-Type": "application/json", Cookie: cookie },
      body: JSON.stringify({
        key: "e2e-test",
        name: "E2E Test",
        description: "Shared universe for Playwright board fixture tests",
      }),
    });
    // 409 Conflict (already exists) is expected and fine — no throw.
  } catch {
    // Non-fatal: tests fall back to the fixture's inline auth + universe check.
  }
}

async function isServerHealthy(): Promise<boolean> {
  try {
    const res = await fetch(HEALTH_URL);
    if (!res.ok) return false;
    const body = await res.json();
    return body?.status === "ok";
  } catch {
    return false;
  }
}

async function waitForServer(): Promise<void> {
  const start = Date.now();
  while (Date.now() - start < MAX_WAIT_MS) {
    if (await isServerHealthy()) return;
    await new Promise((r) => setTimeout(r, POLL_INTERVAL_MS));
  }
  throw new Error(`Server did not start within ${MAX_WAIT_MS}ms`);
}

async function globalSetup(): Promise<void> {
  // If BASE_URL points at an external host (e.g. UAT or prod), skip
  // the local-server bootstrap entirely. The tests will hit the
  // external host directly.
  const baseUrl = process.env.BASE_URL || "";
  const isExternalBase =
    baseUrl &&
    !baseUrl.includes("localhost") &&
    !baseUrl.includes("127.0.0.1") &&
    !baseUrl.includes(`:${SERVER_PORT}`);
  if (isExternalBase) {
    console.log(`Using external base URL: ${baseUrl} (skipping local bootstrap)`);
    process.env.CO_WEB_EXTERNAL = "true";
    return;
  }

  const base = `http://localhost:${SERVER_PORT}`;

  // Check if a healthy server is already running
  if (await isServerHealthy()) {
    console.log("co-web server already running on port", SERVER_PORT);
    process.env.CO_WEB_EXTERNAL = "true";
    await ensureTestUniverse(base);
    return;
  }

  const coWebDir = join(__dirname, "..");
  const projectRoot = join(coWebDir, "..");

  // Build the binary first
  console.log("Building co-web...");
  execSync("cargo build -p co-web", { cwd: projectRoot, stdio: "pipe" });

  const binary = join(projectRoot, "target", "debug", "co-web");
  console.log("Starting co-web server...");

  const server = spawn(
    binary,
    [
      "--port", String(SERVER_PORT),
      "--static-dir", join(coWebDir, "static"),
      "--data", join(coWebDir, "data"),
    ],
    {
      cwd: projectRoot,
      stdio: "pipe",
      env: {
        ...process.env,
        RUST_LOG: "co_web=info",
        // CO-208: disable token-bucket rate limiting so fixture project/universe
        // POSTs don't exhaust the anonymous 5-writes/min cap during e2e runs.
        CO_ENV: process.env.CO_ENV ?? "test",
        CO_BYPASS_RATE_LIMIT: "1",
      },
      detached: true,
    },
  );

  server.unref();

  server.stderr?.on("data", (data: Buffer) => {
    const msg = data.toString();
    if (process.env.DEBUG) console.error("[co-web]", msg.trim());
  });

  server.on("error", (err) => {
    throw new Error(`Failed to start co-web: ${err.message}`);
  });

  // Store PID so teardown can kill it
  process.env.CO_WEB_PID = String(server.pid);

  await waitForServer();
  console.log("co-web server ready on port", SERVER_PORT);
  await ensureTestUniverse(base);
}

export default globalSetup;

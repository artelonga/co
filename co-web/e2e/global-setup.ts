import { execSync, spawn } from "child_process";
import { join } from "path";

const SERVER_PORT = 3000;
const HEALTH_URL = `http://localhost:${SERVER_PORT}/api/health`;
const MAX_WAIT_MS = 60_000;
const POLL_INTERVAL_MS = 500;

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
  // Check if a healthy server is already running
  if (await isServerHealthy()) {
    console.log("co-web server already running on port", SERVER_PORT);
    process.env.CO_WEB_EXTERNAL = "true";
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
      env: { ...process.env, RUST_LOG: "co_web=info" },
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
}

export default globalSetup;

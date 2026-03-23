async function globalTeardown(): Promise<void> {
  // If we used an external server, don't kill it
  if (process.env.CO_WEB_EXTERNAL === "true") {
    console.log("Leaving external co-web server running");
    return;
  }

  const pid = process.env.CO_WEB_PID;
  if (!pid) return;

  try {
    process.kill(Number(pid), "SIGTERM");
    console.log("Stopped co-web server (PID", pid + ")");
  } catch {
    // already dead
  }
}

export default globalTeardown;

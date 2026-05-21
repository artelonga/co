// Read CO credentials from ~/.config/co/credentials (TOML)
// This lets the VS Code extension share auth state with the `co` CLI.

import * as fs from "fs";
import * as os from "os";
import * as path from "path";

import type { CoCredentials, CoProfile } from "./types";

export function credentialsPath(): string {
  const configDir =
    process.env["XDG_CONFIG_HOME"] ?? path.join(os.homedir(), ".config");
  return path.join(configDir, "co", "credentials");
}

/**
 * Minimal TOML section parser — handles the flat key=value format
 * produced by `toml::to_string_pretty` for our Profile structs.
 */
export function parseCredentialsToml(raw: string): CoCredentials {
  const result: CoCredentials = {};
  let currentSection: string | null = null;

  for (const rawLine of raw.split("\n")) {
    const line = rawLine.trim();
    if (!line || line.startsWith("#")) continue;

    // Section header e.g. [default]
    if (line.startsWith("[") && line.endsWith("]")) {
      currentSection = line.slice(1, -1).trim();
      if (currentSection && !(currentSection in result)) {
        result[currentSection] = {};
      }
      continue;
    }

    if (!currentSection) continue;

    const eqIdx = line.indexOf("=");
    if (eqIdx === -1) continue;

    const key = line.slice(0, eqIdx).trim();
    let value = line.slice(eqIdx + 1).trim();
    // Strip surrounding quotes
    if (
      (value.startsWith('"') && value.endsWith('"')) ||
      (value.startsWith("'") && value.endsWith("'"))
    ) {
      value = value.slice(1, -1);
    }
    (result[currentSection] as Record<string, string>)[key] = value;
  }

  return result;
}

/** Load the `default` profile from ~/.config/co/credentials, or null. */
export function loadDefaultProfile(filePath?: string): CoProfile | null {
  const p = filePath ?? credentialsPath();
  let raw: string;
  try {
    raw = fs.readFileSync(p, "utf-8");
  } catch {
    return null;
  }
  const creds = parseCredentialsToml(raw);
  const profile = creds["default"];
  return profile && Object.keys(profile).length > 0 ? profile : null;
}

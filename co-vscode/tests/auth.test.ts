import * as fs from "fs";
import * as os from "os";
import * as path from "path";

import {
  credentialsPath,
  loadDefaultProfile,
  parseCredentialsToml,
} from "../src/auth";

const SAMPLE_TOML = `
[default]
base_url = "https://co.artelonga.com.br"
token = "co_abc123xyz"
token_id = "tok_001"
email = "yuri@example.com"
display_name = "Yuri"

[staging]
base_url = "https://staging.co.artelonga.com.br"
token = "co_staging456"
`;

describe("parseCredentialsToml", () => {
  it("parses the default section", () => {
    const creds = parseCredentialsToml(SAMPLE_TOML);
    expect(creds["default"]).toBeDefined();
    expect(creds["default"]?.token).toBe("co_abc123xyz");
    expect(creds["default"]?.base_url).toBe("https://co.artelonga.com.br");
    expect(creds["default"]?.email).toBe("yuri@example.com");
    expect(creds["default"]?.display_name).toBe("Yuri");
  });

  it("parses multiple sections", () => {
    const creds = parseCredentialsToml(SAMPLE_TOML);
    expect(creds["staging"]?.token).toBe("co_staging456");
  });

  it("returns empty object for empty input", () => {
    expect(parseCredentialsToml("")).toEqual({});
  });

  it("ignores comment lines", () => {
    const creds = parseCredentialsToml("[default]\n# comment\ntoken = \"co_x\"\n");
    expect(creds["default"]?.token).toBe("co_x");
  });

  it("handles missing quotes (bare values)", () => {
    const creds = parseCredentialsToml("[default]\nbase_url = bare_value\n");
    expect(creds["default"]?.base_url).toBe("bare_value");
  });
});

describe("credentialsPath", () => {
  it("returns a path ending in co/credentials", () => {
    const p = credentialsPath();
    expect(p.endsWith(path.join("co", "credentials"))).toBe(true);
  });

  it("uses XDG_CONFIG_HOME if set", () => {
    const orig = process.env["XDG_CONFIG_HOME"];
    process.env["XDG_CONFIG_HOME"] = "/custom/config";
    const p = credentialsPath();
    expect(p).toBe(path.join("/custom/config", "co", "credentials"));
    if (orig !== undefined) {
      process.env["XDG_CONFIG_HOME"] = orig;
    } else {
      delete process.env["XDG_CONFIG_HOME"];
    }
  });
});

describe("loadDefaultProfile", () => {
  let tmpFile: string;

  beforeEach(() => {
    tmpFile = path.join(os.tmpdir(), `co-creds-test-${Date.now()}.toml`);
  });

  afterEach(() => {
    try { fs.unlinkSync(tmpFile); } catch { /* ignore */ }
  });

  it("returns null when file does not exist", () => {
    expect(loadDefaultProfile("/nonexistent/path")).toBeNull();
  });

  it("loads the default profile from a temp file", () => {
    fs.writeFileSync(tmpFile, SAMPLE_TOML, "utf-8");
    const profile = loadDefaultProfile(tmpFile);
    expect(profile).not.toBeNull();
    expect(profile?.token).toBe("co_abc123xyz");
    expect(profile?.email).toBe("yuri@example.com");
  });

  it("returns null when default section is absent", () => {
    fs.writeFileSync(tmpFile, "[other]\ntoken = \"co_x\"\n", "utf-8");
    expect(loadDefaultProfile(tmpFile)).toBeNull();
  });
});

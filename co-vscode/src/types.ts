// CO VS Code extension — shared types (mirrors vault_routes.rs + models.rs)

export interface VaultStat {
  ctime: number; // ms since epoch
  mtime: number; // ms since epoch
  size: number;
}

export interface VaultFileInfo {
  path: string;
  stat: VaultStat;
}

export interface VaultFile {
  path: string;
  content: string;
  frontmatter: Record<string, unknown>;
  tags: string[];
  stat: VaultStat;
}

export interface UniverseInfo {
  key: string;
  name: string;
  description?: string;
  visibility: string;
}

export interface MeUniversesResponse {
  owned: UniverseInfo[];
  member: UniverseInfo[];
  subscribed: UniverseInfo[];
  counts: { owned: number; member: number; subscribed: number; invited: number; discoverable: number };
}

// Credentials matching ~/.config/co/credentials TOML Profile struct
export interface CoProfile {
  base_url?: string;
  session?: string;
  session_expires_at?: string;
  token?: string;
  token_id?: string;
  expires_at?: string;
  user_id?: string;
  email?: string;
  display_name?: string;
}

export type CoCredentials = Record<string, CoProfile>;

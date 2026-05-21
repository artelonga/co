// CO VS Code Extension — entry point
//
// Registers the co:// FileSystemProvider so VS Code can open CO universes
// as remote workspaces. Also provides wikilink completion for markdown files.

import * as vscode from "vscode";

import { CoApiClient } from "./api";
import { loadDefaultProfile } from "./auth";
import { CoFileSystemProvider } from "./coFileSystemProvider";
import { WikilinkCompletionProvider } from "./completions";

export function activate(ctx: vscode.ExtensionContext): void {
  const result = createClientAndProvider();
  if (!result) {
    // No token — extension registers commands but the provider isn't active yet.
    ctx.subscriptions.push(
      vscode.commands.registerCommand("co.openRemoteWorkspace", async () => {
        vscode.window.showErrorMessage(
          'CO: No API token configured. Run `co auth login` or set "co.apiToken" in settings.',
        );
      }),
      vscode.commands.registerCommand("co.configure", () => {
        vscode.commands.executeCommand(
          "workbench.action.openSettings",
          "@ext:artelonga.co-vscode",
        );
      }),
    );
    return;
  }

  const { client, provider } = result;

  ctx.subscriptions.push(
    vscode.workspace.registerFileSystemProvider("co", provider, {
      isCaseSensitive: true,
    }),

    vscode.languages.registerCompletionItemProvider(
      { scheme: "co", language: "markdown" },
      new WikilinkCompletionProvider(provider),
      "[",
    ),

    vscode.commands.registerCommand(
      "co.openRemoteWorkspace",
      async () => openRemoteWorkspace(client),
    ),

    vscode.commands.registerCommand("co.configure", () => {
      vscode.commands.executeCommand(
        "workbench.action.openSettings",
        "@ext:artelonga.co-vscode",
      );
    }),
  );
}

export function deactivate(): void {}

// ─── Helpers ─────────────────────────────────────────────────────────────────

function resolveCredentials(): { instanceUrl: string; apiToken: string } | null {
  const config = vscode.workspace.getConfiguration("co");
  const instanceUrl =
    (config.get<string>("instanceUrl") ?? "").replace(/\/$/, "") ||
    "https://co.artelonga.com.br";
  let apiToken = config.get<string>("apiToken") ?? "";

  if (!apiToken) {
    const profile = loadDefaultProfile();
    if (profile?.token) {
      apiToken = profile.token;
    }
  }

  if (!apiToken) return null;
  return { instanceUrl, apiToken };
}

function createClientAndProvider(): {
  client: CoApiClient;
  provider: CoFileSystemProvider;
} | null {
  const creds = resolveCredentials();
  if (!creds) {
    vscode.window.showWarningMessage(
      'CO: No API token found. Run `co auth login` or set "co.apiToken" in settings.',
    );
    return null;
  }
  const client = new CoApiClient(creds.instanceUrl, creds.apiToken);
  return { client, provider: new CoFileSystemProvider(client) };
}

async function openRemoteWorkspace(client: CoApiClient): Promise<void> {
  let universes: Array<{ label: string; description: string; key: string }>;

  try {
    const resp = await client.getMyUniverses();
    const all = [...resp.owned, ...resp.member, ...resp.subscribed];
    universes = all.map((u) => ({
      label: u.name,
      description: u.key,
      key: u.key,
    }));
  } catch (e) {
    vscode.window.showErrorMessage(
      `CO: Failed to list universes — ${(e as Error).message}`,
    );
    return;
  }

  if (universes.length === 0) {
    vscode.window.showInformationMessage(
      "CO: No universes found for your account.",
    );
    return;
  }

  const picked = await vscode.window.showQuickPick(universes, {
    placeHolder: "Select a CO universe to open as workspace",
    matchOnDescription: true,
  });
  if (!picked) return;

  const folderUri = vscode.Uri.parse(`co://${picked.key}/`);
  vscode.workspace.updateWorkspaceFolders(
    vscode.workspace.workspaceFolders?.length ?? 0,
    0,
    { uri: folderUri, name: `CO: ${picked.label}` },
  );
}

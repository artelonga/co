// Wikilink auto-completion for CO markdown files.
// Triggers on "[[" and returns entries from the universe's vault.

import * as vscode from "vscode";

import { parseUri } from "./coFileSystemProvider";
import type { CoFileSystemProvider } from "./coFileSystemProvider";

export class WikilinkCompletionProvider
  implements vscode.CompletionItemProvider
{
  constructor(private readonly provider: CoFileSystemProvider) {}

  async provideCompletionItems(
    document: vscode.TextDocument,
    position: vscode.Position,
  ): Promise<vscode.CompletionItem[]> {
    if (document.uri.scheme !== "co") return [];

    // Only trigger after "[["
    const linePrefix = document
      .lineAt(position)
      .text.slice(0, position.character);
    if (!linePrefix.endsWith("[[")) return [];

    const { slug } = parseUri(document.uri);

    try {
      const files = await this.provider.listFilesForSlug(slug);
      return files
        .filter((f) => f.path.endsWith(".md"))
        .map((f) => {
          const name = f.path.replace(/\.md$/, "");
          const item = new vscode.CompletionItem(
            name,
            vscode.CompletionItemKind.File,
          );
          item.insertText = `${name}]]`;
          item.filterText = name;
          item.detail = f.path;
          return item;
        });
    } catch {
      return [];
    }
  }
}

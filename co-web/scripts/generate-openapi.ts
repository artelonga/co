/**
 * Generate openapi.yaml from the route catalog.
 *
 * Usage:
 *   npm run openapi:gen    — validate + regenerate openapi.yaml
 *   npm run openapi:check  — validate only (exits 1 on drift, no file write)
 *
 * The catalog (docs/architecture/api-catalog.md) is the single source of truth.
 * The script also walks co-web/src/**\/*_routes.rs to detect routes that exist
 * in code but are missing from the catalog.
 *
 * Failure modes on --check:
 *   + Route in code but missing from catalog: POST /api/v1/me/graph-views
 *   - Route in catalog but not found in code: GET /api/v1/legacy/foo
 *   ~ Generated openapi.yaml differs from on-disk file (run `npm run openapi:gen`)
 */

import { readFileSync, writeFileSync, readdirSync, statSync, existsSync } from 'node:fs';
import { join, dirname, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const WEB_DIR = join(__dirname, '..');
const REPO_ROOT = join(WEB_DIR, '..');
const CATALOG_PATH = join(REPO_ROOT, 'docs', 'architecture', 'api-catalog.md');
const OPENAPI_OUT = join(WEB_DIR, 'openapi.yaml');
const COMPONENTS_PATH = join(WEB_DIR, 'openapi-components.yaml');
const SRC_DIR = join(WEB_DIR, 'src');

const CHECK_MODE = process.argv.includes('--check');
const HTTP_METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface CatalogEntry {
  method: string;
  path: string;   // absolute, e.g. /api/v1/auth/login
  auth: string;
  summary: string;
  tags: string[];
}

interface RouteEntry {
  method: string;
  path: string;   // may be partial (relative to axum .nest() prefix)
  file: string;
}

// ---------------------------------------------------------------------------
// 1. Parse api-catalog.md
// ---------------------------------------------------------------------------

function sectionBasePrefix(heading: string): string {
  // "## entries — `/api/v1/universes/{slug}/...`" → "/api/v1/universes"
  const m = heading.match(/`(\/[^`]+)`/);
  if (!m) return '';
  const base = m[1].split('{')[0].replace(/\/$/, '');
  return base.startsWith('/') ? base : '';
}

function resolveCatalogPath(raw: string, sectionPrefix: string): string {
  const path = raw.replace(/`/g, '').trim();
  // Already absolute
  if (path.match(/^\/(api|oauth|\.well-known|v1|auth)\//)) return path;
  // Relative path starting with /{param} — apply section prefix
  if (sectionPrefix && path.startsWith('/{')) return sectionPrefix + path;
  return path;
}

const SKIP_SECTIONS = new Set([
  'SPA + page routes (server.rs)',
  // CO-210: server-rendered docs viewer — literal SPA-style page routes
  // (/seguranca, /licensa, …), no API handler, so excluded from the API drift.
  'docs pages',
  'WebSockets',
  'proposals — see entries section above',
  'vault — see vault section above',
  'vault — see assets section above',
]);

function parseCatalog(md: string): CatalogEntry[] {
  const entries: CatalogEntry[] = [];
  let section = '';
  let prefix = '';
  let inTable = false;
  let pastHeader = false;

  for (const raw of md.split('\n')) {
    const line = raw.trim();

    if (line.startsWith('## ')) {
      section = line.slice(3).split('—')[0].split('(')[0].trim();
      prefix = sectionBasePrefix(line);
      inTable = false;
      pastHeader = false;
      continue;
    }

    if (SKIP_SECTIONS.has(section)) continue;

    if (/^\|\s*Method\s*\|/i.test(line)) {
      inTable = true;
      pastHeader = false;
      continue;
    }

    if (inTable && /^\|[-\s|]+\|/.test(line)) {
      pastHeader = true;
      continue;
    }

    if (inTable && pastHeader && line.startsWith('|')) {
      const cells = line.split('|').map(c => c.trim()).filter(Boolean);
      if (cells.length < 4) continue;

      const rawMethod = cells[0].toUpperCase();
      if (rawMethod.startsWith('WS')) continue;

      // Handle compound: "GET/PUT/DELETE"
      const methods = rawMethod.split('/').filter(m => HTTP_METHODS.includes(m));
      if (methods.length === 0) continue;

      const path = resolveCatalogPath(cells[1], prefix);
      const auth = cells[2];
      const summary = cells[3];

      for (const method of methods) {
        entries.push({ method, path, auth, summary, tags: [section] });
      }
    } else if (inTable && pastHeader && line && !line.startsWith('|')) {
      inTable = false;
    }
  }

  return entries;
}

// ---------------------------------------------------------------------------
// 2. Walk *_routes.rs to discover code routes
// ---------------------------------------------------------------------------

// SPA page route partial paths — only appear as-is in server/router.rs.
// Other files (e.g. asset_routes.rs, storage_dashboard.rs) legitimately use
// the same partial strings for API routes under a different prefix.
const SPA_PATHS = new Set([
  '/', '/{slug}', '/{slug}/', '/{slug}/{*subpath}',
  '/admin', '/admin/deployments', '/admin/leads.html',
  '/repl', '/storage', '/analytics', '/changelog',
  '/co/telemetria', '/co/co-dev/pipeline', '/settings/sync',
  '/yggdrasil/{game}', '/notifications', '/recover',
  '/invitations/{token}', '/{slug}/assets', '/{slug}/graph',
  '/linhadotempo', '/timeline',
]);

// Catalog paths whose code routes resolve via "/" in a nested router.
// Static suffix analysis can't verify these; they are reviewed manually.
const UNCHECKED_CATALOG_PATHS = new Set([
  '/api/v1/universes',
  '/api/v1/universes/',
  '/api/v1/interactions',
  '/api/v1/interactions/',
]);

function isApiRouteForCheck(path: string, file: string): boolean {
  // SPA routes live exclusively in server/router.rs — exclude only from there
  if (SPA_PATHS.has(path) && file === 'server/router.rs') return false;
  if (path.endsWith('/ws') || path.includes('/ws/')) return false; // WebSocket
  if (path === '/api/v1/sync/ws') return false;
  if (path === '/api/v1/analytics/stream') return false; // SSE/WS stream
  // Root routes ("/") in nested routers can't be suffix-matched — skip code-side
  if (path === '/') return false;
  // Test/extractor helper routes
  if (path === '/test') return false;
  if (path === '/_schema_check') return false;
  // Skip files that are not route modules (e.g. middleware extractors)
  if (file.includes('extractors.rs') && !file.includes('routes')) return false;
  // CO-210: docs_routes.rs is entirely SPA-style page routes (the docs viewer +
  // its /seguranca/{*rest} catch-all) — not API; catalogued under the skipped
  // "docs pages" section, so exclude it from the code-side API check too.
  if (file.includes('docs_routes.rs')) return false;
  return true;
}

function findRouteFiles(dir: string): string[] {
  const files: string[] = [];
  for (const entry of readdirSync(dir).sort()) {
    const full = join(dir, entry);
    if (statSync(full).isDirectory()) {
      files.push(...findRouteFiles(full));
    } else if (entry.endsWith('.rs') && !entry.endsWith('_test.rs') && entry !== 'mod.rs') {
      files.push(full);
    }
  }
  return files;
}

function extractRoutes(content: string, filename: string): RouteEntry[] {
  const results: RouteEntry[] = [];
  let pos = 0;

  while (pos < content.length) {
    const idx = content.indexOf('.route(', pos);
    if (idx === -1) break;

    // Extract content between the parens of .route(...)
    let depth = 1;
    let i = idx + '.route('.length;
    while (i < content.length && depth > 0) {
      if (content[i] === '(') depth++;
      else if (content[i] === ')') depth--;
      if (depth > 0) i++;
    }
    const inner = content.slice(idx + '.route('.length, i);

    // Path is the first string literal
    const pathMatch = inner.match(/^\s*"([^"]+)"/);
    if (pathMatch) {
      const path = pathMatch[1];
      for (const method of HTTP_METHODS) {
        if (new RegExp(`\\b${method.toLowerCase()}\\s*\\(`).test(inner)) {
          results.push({ method, path, file: filename });
        }
      }
    }

    pos = idx + 1;
  }

  return results;
}

function walkRoutes(srcDir: string): RouteEntry[] {
  const results: RouteEntry[] = [];
  for (const file of findRouteFiles(srcDir)) {
    const content = readFileSync(file, 'utf8');
    // Only process files that contain .route( calls
    if (!content.includes('.route(')) continue;
    const rel = relative(srcDir, file);
    const routes = extractRoutes(content, rel).filter(r => isApiRouteForCheck(r.path, rel));
    results.push(...routes);
  }
  return results;
}

// ---------------------------------------------------------------------------
// 3. Drift detection
// ---------------------------------------------------------------------------

function normalizeSegments(path: string): string[] {
  return path.replace(/\{[^}]+\}/g, '{p}').split('/').filter(Boolean);
}

function segmentSuffixMatch(catalogPath: string, codePath: string): boolean {
  const cat = normalizeSegments(catalogPath);
  const code = normalizeSegments(codePath);
  if (code.length === 0 || code.length > cat.length) return false;
  const suffix = cat.slice(cat.length - code.length);
  return suffix.join('/') === code.join('/');
}

function entryKey(method: string, path: string) {
  return `${method} ${path}`;
}

interface DriftResult {
  missingFromCatalog: string[];  // in code, not in catalog
  missingFromCode: string[];     // in catalog, not in code
  yamlStale: boolean;
}

function detectDrift(
  catalog: CatalogEntry[],
  codeRoutes: RouteEntry[],
  generatedYaml: string,
): DriftResult {
  const missingFromCatalog: string[] = [];
  const missingFromCode: string[] = [];

  // Build catalog key set for quick lookup
  const catalogKeys = new Set(catalog.map(e => entryKey(e.method, e.path)));

  // Routes in code but not in catalog (suffix matching)
  const checked = new Set<string>();
  for (const route of codeRoutes) {
    const key = entryKey(route.method, route.path);
    if (checked.has(key)) continue;
    checked.add(key);

    const hasMatch = catalog.some(
      c => c.method === route.method && segmentSuffixMatch(c.path, route.path),
    );
    if (!hasMatch) {
      missingFromCatalog.push(`  + ${route.method} ${route.path}  (${route.file})`);
    }
  }

  // Routes in catalog but not found in code
  for (const entry of catalog) {
    // Skip paths whose code uses "/" at the nest root — can't suffix-match statically
    if (UNCHECKED_CATALOG_PATHS.has(entry.path)) continue;
    const foundInCode = codeRoutes.some(
      r => r.method === entry.method && segmentSuffixMatch(entry.path, r.path),
    );
    if (!foundInCode) {
      missingFromCode.push(`  - ${entry.method} ${entry.path}`);
    }
  }

  let yamlStale = false;
  if (existsSync(OPENAPI_OUT)) {
    const onDisk = readFileSync(OPENAPI_OUT, 'utf8');
    yamlStale = onDisk.trim() !== generatedYaml.trim();
  } else {
    yamlStale = true;
  }

  return { missingFromCatalog, missingFromCode, yamlStale };
}

// ---------------------------------------------------------------------------
// 4. Generate openapi.yaml
// ---------------------------------------------------------------------------

const METHOD_ORDER: Record<string, number> = {
  GET: 0, POST: 1, PUT: 2, PATCH: 3, DELETE: 4,
};

function authToSecurity(auth: string): string {
  if (!auth || auth === 'anon' || auth === '-' || auth === 'anon (in-handler)') return '';
  if (auth.startsWith('shared-secret')) return '      security:\n        - sharedSecret: []\n';
  return '      security:\n        - sessionCookie: []\n';
}

function pathParams(path: string): string {
  const params = [...path.matchAll(/\{([^}*]+)\}/g)].map(m => m[1]);
  if (params.length === 0) return '';
  const lines = params.map(
    p =>
      `        - name: ${p}\n          in: path\n          required: true\n          schema:\n            type: string`,
  );
  return `      parameters:\n${lines.join('\n')}\n`;
}

function buildPathsYaml(catalog: CatalogEntry[]): string {
  const byPath = new Map<string, CatalogEntry[]>();
  for (const e of catalog) {
    const key = e.path;
    if (!byPath.has(key)) byPath.set(key, []);
    byPath.get(key)!.push(e);
  }

  const sortedPaths = [...byPath.keys()].sort();
  const lines: string[] = ['paths:'];
  for (const path of sortedPaths) {
    lines.push(`  ${path}:`);
    const ops = byPath
      .get(path)!
      .sort((a, b) => (METHOD_ORDER[a.method] ?? 9) - (METHOD_ORDER[b.method] ?? 9));
    for (const op of ops) {
      const safeTag = op.tags[0].replace(/"/g, '\\"');
      const safeSummary = op.summary.replace(/"/g, '\\"');
      lines.push(`    ${op.method.toLowerCase()}:`);
      lines.push(`      tags: ["${safeTag}"]`);
      lines.push(`      summary: "${safeSummary}"`);
      const security = authToSecurity(op.auth);
      if (security) lines.push(security.trimEnd());
      const params = pathParams(path);
      if (params) lines.push(params.trimEnd());
      lines.push('      responses:');
      lines.push('        "200":');
      lines.push('          description: OK');
    }
  }
  return lines.join('\n');
}

function buildTagsYaml(catalog: CatalogEntry[]): string {
  const seen = new Set<string>();
  const tags: string[] = [];
  for (const e of catalog) {
    for (const t of e.tags) {
      if (!seen.has(t)) {
        seen.add(t);
        tags.push(t);
      }
    }
  }
  const lines = ['tags:'];
  for (const t of tags) {
    lines.push(`  - name: "${t}"`);
    lines.push(`    description: "${t} endpoints"`);
  }
  return lines.join('\n');
}

const OPENAPI_HEADER = `\
openapi: 3.1.0
info:
  title: CO Web API
  description: |
    API da plataforma CO — gestão de conteúdo em grafo.

    Gerado automaticamente a partir de docs/architecture/api-catalog.md.
    Para adicionar ou remover endpoints, atualize o catálogo e rode
    \`npm run openapi:gen\` em co-web/.

    ## Como adicionar uma rota

    1. Implemente o handler e registre-o no arquivo \`*_routes.rs\` adequado.
    2. Adicione uma linha na tabela correspondente em \`docs/architecture/api-catalog.md\`.
    3. Rode \`cd co-web && npm run openapi:gen\` para regenerar este arquivo.
  version: "2.40.0"
  contact:
    name: Arte Longa
    email: yuri@artelonga.com.br
  license:
    name: MIT

servers:
  - url: https://co-artelonga.fly.dev
    description: Production
  - url: http://localhost:8742
    description: Local
`;

function buildOpenApi(catalog: CatalogEntry[], componentsYaml: string): string {
  const tags = buildTagsYaml(catalog);
  const paths = buildPathsYaml(catalog);
  return `${OPENAPI_HEADER}\n${tags}\n\n${paths}\n${componentsYaml ? '\n' + componentsYaml : ''}`;
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

function main() {
  console.log('[generate-openapi] Reading catalog:', CATALOG_PATH);
  const catalogMd = readFileSync(CATALOG_PATH, 'utf8');
  const catalog = parseCatalog(catalogMd);
  console.log(`[generate-openapi]   ${catalog.length} catalog entries`);

  console.log('[generate-openapi] Walking routes:', SRC_DIR);
  const codeRoutes = walkRoutes(SRC_DIR);
  console.log(`[generate-openapi]   ${codeRoutes.length} route handlers found in code`);

  let componentsYaml = '';
  if (existsSync(COMPONENTS_PATH)) {
    componentsYaml = readFileSync(COMPONENTS_PATH, 'utf8');
  }

  const generated = buildOpenApi(catalog, componentsYaml);
  const drift = detectDrift(catalog, codeRoutes, generated);

  let hasDrift = false;

  if (drift.missingFromCatalog.length > 0) {
    console.error('\n[generate-openapi] DRIFT detected:');
    console.error('  Routes in code but MISSING from catalog (add them to api-catalog.md):');
    drift.missingFromCatalog.forEach(l => console.error(l));
    hasDrift = true;
  }

  if (drift.missingFromCode.length > 0) {
    if (!hasDrift) console.error('\n[generate-openapi] DRIFT detected:');
    console.error('  Routes in catalog but NOT found in code (remove or update them):');
    drift.missingFromCode.forEach(l => console.error(l));
    hasDrift = true;
  }

  if (drift.yamlStale && CHECK_MODE) {
    if (!hasDrift) console.error('\n[generate-openapi] DRIFT detected:');
    console.error(
      '  ~ openapi.yaml is stale — run `cd co-web && npm run openapi:gen` to update',
    );
    hasDrift = true;
  }

  if (CHECK_MODE) {
    if (!hasDrift) {
      console.log(
        `[generate-openapi] ✓ No drift — catalog (${catalog.length} entries) matches code and openapi.yaml`,
      );
    }
    process.exit(hasDrift ? 1 : 0);
  }

  // Gen mode: write yaml
  writeFileSync(OPENAPI_OUT, generated, 'utf8');
  console.log(`[generate-openapi] ✓ openapi.yaml written (${generated.length} bytes)`);

  if (hasDrift) {
    console.warn('[generate-openapi] ⚠  Drift remains — update the catalog to address the issues above');
    process.exit(1);
  }
}

main();

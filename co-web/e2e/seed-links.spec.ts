import { test, expect, request } from "@playwright/test";

/**
 * Crawl the seeded template content via the public entries API and
 * assert every internal/external link resolves. Catches the kind of
 * regression where a seeded page references a slug or external URL
 * that 404s — e.g. the Fly DPA link rot fixed in 2.7.7.
 *
 * Run against the deployed host:
 *   BASE_URL=https://co-artelonga.fly.dev npx playwright test e2e/seed-links.spec.ts
 */

const SEED_PAGE_SLUGS = [
  "sobre",
  "termos",
  "privacidade",
  "dados-rastreados",
  "linhas-do-tempo",
  "co-plataforma",
  "guia",
  "seguranca",
  "seguranca-dependencias",
  "seguranca-cenarios",
  "seguranca-vapid",
  "licensa",
  "renderers",
  "infra",
  "infra-co",
  "infra-yggdrasil",
  "infra-quilomboaraucaria",
  "infra-rfq-gateway",
];

const KNOWN_UNIVERSE_SLUGS = ["template", "co", "yggdrasil", "tempo", "humanity", "universo"];

// External hosts that may rate-limit or block automated HEAD probes —
// the link itself can be valid but the test sees 403/429. Whitelist
// them so we don't fail on the noise. Anything outside this list is
// expected to return 2xx/3xx.
const EXTERNAL_FLAKY_HOSTS = new Set<string>([
  // (empty for now — populate when we hit a flaky host)
]);

function extractMarkdownLinks(body: string): string[] {
  const out: string[] = [];
  const re = /\[([^\]]+)\]\(([^)]+)\)/g;
  let m: RegExpExecArray | null;
  while ((m = re.exec(body)) !== null) {
    out.push(m[2].trim());
  }
  // Also catch <https://...> autolinks
  const auto = /<(https?:\/\/[^>]+)>/g;
  while ((m = auto.exec(body)) !== null) {
    out.push(m[1].trim());
  }
  return out;
}

function classify(url: string): "external" | "internal" | "anchor" | "other" {
  if (url.startsWith("http://") || url.startsWith("https://")) return "external";
  if (url.startsWith("#")) return "anchor";
  if (url.startsWith("/")) return "internal";
  if (url.startsWith("?")) return "internal";
  return "other";
}

test.describe("Seed page links resolve", () => {
  test("every link in every seeded template page returns 2xx/3xx", async () => {
    const base = process.env.BASE_URL ?? "https://co-artelonga.fly.dev";
    const ctx = await request.newContext({ baseURL: base });

    // 1. Pull each seed page body via the public entries API.
    //    Throttle gently to stay under the prod rate limiter.
    const sleepMs = (ms: number) => new Promise((r) => setTimeout(r, ms));
    const bodies = new Map<string, string>();
    for (const slug of SEED_PAGE_SLUGS) {
      const apiPath = `/api/v1/universes/template/entries/content/${slug}.md`;
      const res = await ctx.get(apiPath);
      expect(res.status(), `entry ${slug} should be 200`).toBe(200);
      const json = await res.json();
      bodies.set(slug, json.body || "");
      await sleepMs(40);
    }
    // Index lives at index.md, not content/index.md.
    const idxRes = await ctx.get("/api/v1/universes/template/entries/index.md");
    expect(idxRes.status()).toBe(200);
    bodies.set("index", (await idxRes.json()).body || "");

    // 2. Collect unique links and classify.
    type LinkRef = { source: string; url: string; kind: ReturnType<typeof classify> };
    const refs: LinkRef[] = [];
    for (const [slug, body] of bodies) {
      for (const url of extractMarkdownLinks(body)) {
        refs.push({ source: slug, url, kind: classify(url) });
      }
    }
    const totalLinks = refs.length;
    expect(totalLinks).toBeGreaterThan(0);

    // 3. Verify each link. Internal links go through the API; external
    //    links use a HEAD with redirect-follow.
    const failures: string[] = [];

    // De-duplicate URLs so we don't re-fetch the same link 5x just because
    // 5 pages reference it. Cuts request volume by ~5x and avoids hitting
    // the rate limiter on prod.
    const seenUrls = new Set<string>();

    for (const ref of refs) {
      if (ref.kind === "anchor" || ref.kind === "other") continue;
      if (seenUrls.has(ref.url)) continue;
      seenUrls.add(ref.url);
      // Throttle to stay under the rate-limit (CO-80 default: 1000/min).
      await sleepMs(40);

      if (ref.kind === "internal") {
        // Two internal shapes today:
        //   /<universe>/<entry>           → /api/v1/universes/<universe>/entries/<...>.md
        //   /<universe>?page=<slug>       → /api/v1/universes/<universe>/entries/content/<slug>.md
        //   /<universe>?path=<full-path>  → /api/v1/universes/<universe>/entries/<full-path>
        let target = "";
        const qIdx = ref.url.indexOf("?");
        const pathPart = qIdx >= 0 ? ref.url.slice(0, qIdx) : ref.url;
        const queryPart = qIdx >= 0 ? ref.url.slice(qIdx + 1) : "";
        const segs = pathPart.replace(/^\//, "").split("/");

        if (segs[0] === "co" && segs.length >= 2 && KNOWN_UNIVERSE_SLUGS.includes(segs[1])) {
          const universe = segs[1];
          if (queryPart) {
            const params = new URLSearchParams(queryPart);
            const page = params.get("page");
            const pathParam = params.get("path");
            if (page) target = `/api/v1/universes/${universe}/entries/content/${page}.md`;
            else if (pathParam) target = `/api/v1/universes/${universe}/entries/${pathParam}`;
          }
          if (!target && segs.length >= 3) {
            target = `/api/v1/universes/${universe}/entries/${segs.slice(2).join("/")}`;
          }
          if (!target) target = `/api/v1/universes/${universe}`;
        } else if (KNOWN_UNIVERSE_SLUGS.includes(segs[0])) {
          const universe = segs[0];
          if (queryPart) {
            const params = new URLSearchParams(queryPart);
            const page = params.get("page");
            const pathParam = params.get("path");
            if (page) target = `/api/v1/universes/${universe}/entries/content/${page}.md`;
            else if (pathParam) target = `/api/v1/universes/${universe}/entries/${pathParam}`;
          }
          if (!target && segs.length >= 2) {
            target = `/api/v1/universes/${universe}/entries/${segs.slice(1).join("/")}`;
          }
          if (!target) target = `/api/v1/universes/${universe}`;
        } else {
          // Pretty URL like /seguranca — expect 307/302 redirect.
          target = pathPart;
        }

        const res = await ctx.get(target, { maxRedirects: 5 });
        const code = res.status();
        if (code < 200 || code >= 400) {
          failures.push(`[${ref.source}] internal ${ref.url} → ${target} → ${code}`);
        }
        continue;
      }

      // External
      try {
        const u = new URL(ref.url);
        if (EXTERNAL_FLAKY_HOSTS.has(u.host)) continue;
      } catch (_) {
        failures.push(`[${ref.source}] external ${ref.url} → unparseable`);
        continue;
      }

      const extCtx = await request.newContext({});
      try {
        // Some hosts disallow HEAD — fall back to GET.
        let res = await extCtx.fetch(ref.url, {
          method: "HEAD",
          maxRedirects: 5,
          timeout: 10_000,
        });
        if (res.status() >= 400) {
          res = await extCtx.fetch(ref.url, {
            method: "GET",
            maxRedirects: 5,
            timeout: 15_000,
          });
        }
        const code = res.status();
        // 4xx/5xx fails. 200/301/302 pass.
        if (code >= 400) {
          failures.push(`[${ref.source}] external ${ref.url} → ${code}`);
        }
      } catch (e) {
        const msg = e instanceof Error ? e.message : String(e);
        failures.push(`[${ref.source}] external ${ref.url} → ${msg}`);
      } finally {
        await extCtx.dispose();
      }
    }

    await ctx.dispose();

    if (failures.length > 0) {
      throw new Error(
        `${failures.length}/${totalLinks} links failed:\n  ${failures.join("\n  ")}`
      );
    }
  });

  test("popular endpoint returns at least 5 entries for template universe", async () => {
    const base = process.env.BASE_URL ?? "https://co-artelonga.fly.dev";
    const ctx = await request.newContext({ baseURL: base });

    const res = await ctx.get(
      "/api/v1/universes/template/entries/popular?limit=5"
    );
    expect(res.status(), "popular endpoint should be 200").toBe(200);

    const items = await res.json();
    expect(Array.isArray(items), "popular should return an array").toBe(true);
    expect(
      items.length,
      "popular should return at least 5 entries"
    ).toBeGreaterThanOrEqual(5);

    for (const item of items) {
      expect(typeof item.path, "each item has a path").toBe("string");
      expect(typeof item.title, "each item has a title").toBe("string");
    }

    await ctx.dispose();
  });

  test("template tutorial project is keyed TUTORIAL, not CO", async () => {
    const base = process.env.BASE_URL ?? "https://co-artelonga.fly.dev";
    const ctx = await request.newContext({ baseURL: base });

    const res = await ctx.get("/api/v1/universes/template/projects");
    expect(res.status(), "projects endpoint should be 200").toBe(200);

    const projects = await res.json();
    expect(Array.isArray(projects), "projects should be an array").toBe(true);

    const tutorialProject = projects.find(
      (p: { key: string }) => p.key === "TUTORIAL",
    );
    expect(
      tutorialProject,
      "template should have a project keyed TUTORIAL",
    ).toBeDefined();

    const coProject = projects.find(
      (p: { key: string }) => p.key === "CO",
    );
    expect(
      coProject,
      "template should NOT have a project keyed CO (CO belongs to the co universe)",
    ).toBeUndefined();

    await ctx.dispose();
  });

  test("pretty-URL redirects land on /template/<slug>", async ({}) => {
    const base = process.env.BASE_URL ?? "https://co-artelonga.fly.dev";
    const ctx = await request.newContext({ baseURL: base });

    for (const slug of SEED_PAGE_SLUGS) {
      const res = await ctx.get(`/${slug}`, { maxRedirects: 0 });
      const code = res.status();
      // 307 (TemporaryRedirect) or 302 acceptable.
      expect([301, 302, 307], `/${slug} redirect status`).toContain(code);
      const loc = res.headers()["location"] || "";
      expect(loc, `/${slug} location header`).toBe(`/template/${slug}`);
    }

    await ctx.dispose();
  });
});

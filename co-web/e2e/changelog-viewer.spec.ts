import { test, expect } from "@playwright/test";

// CO-260 — Cross-version changelog viewer: range queries + type filter + sort

test.describe("Changelog API", () => {
  test("GET /api/v1/changelog returns valid JSON shape", async ({
    request,
  }) => {
    const res = await request.get("/api/v1/changelog");
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body).toHaveProperty("range");
    expect(body).toHaveProperty("versions");
    expect(Array.isArray(body.versions)).toBe(true);
  });

  test("versions are sorted newest-first by default", async ({ request }) => {
    const res = await request.get("/api/v1/changelog");
    expect(res.status()).toBe(200);
    const { versions } = await res.json();
    if (versions.length < 2) return; // nothing to compare
    // Check that first version >= second version (semver)
    const toTuple = (v: string) =>
      v.split(".").map((n: string) => parseInt(n, 10)) as [
        number,
        number,
        number,
      ];
    const [a, b] = [toTuple(versions[0].version), toTuple(versions[1].version)];
    const gte =
      a[0] > b[0] ||
      (a[0] === b[0] && a[1] > b[1]) ||
      (a[0] === b[0] && a[1] === b[1] && a[2] >= b[2]);
    expect(gte).toBe(true);
  });

  test("?from= and ?to= filter version range", async ({ request }) => {
    // First get all versions to pick a valid from/to
    const all = await request.get("/api/v1/changelog");
    const { versions: allVers } = await all.json();
    if (allVers.length < 2) {
      // Not enough versions to test range; skip gracefully
      return;
    }

    const sorted = [...allVers].sort((a: { version: string }, b: { version: string }) => {
      const toT = (v: string) => v.split(".").map(Number);
      const [at, bt] = [toT(a.version), toT(b.version)];
      return at[0] - bt[0] || at[1] - bt[1] || at[2] - bt[2];
    });

    const from = sorted[0].version;
    const to = sorted[Math.min(1, sorted.length - 1)].version;

    const res = await request.get(
      `/api/v1/changelog?from=${from}&to=${to}`,
    );
    expect(res.status()).toBe(200);
    const { versions } = await res.json();

    // All returned versions must be within [from, to]
    for (const v of versions) {
      const toT = (s: string) => s.split(".").map(Number);
      const [vt, ft, tt] = [
        toT(v.version),
        toT(from),
        toT(to),
      ];
      const gte = vt[0] > ft[0] || (vt[0] === ft[0] && vt[1] > ft[1]) || (vt[0] === ft[0] && vt[1] === ft[1] && vt[2] >= ft[2]);
      const lte = vt[0] < tt[0] || (vt[0] === tt[0] && vt[1] < tt[1]) || (vt[0] === tt[0] && vt[1] === tt[1] && vt[2] <= tt[2]);
      expect(gte && lte).toBe(true);
    }
  });

  test("?type=feat filters to feat entries only", async ({ request }) => {
    const res = await request.get("/api/v1/changelog?type=feat");
    expect(res.status()).toBe(200);
    const { versions } = await res.json();
    for (const v of versions) {
      for (const e of v.entries) {
        expect(e.entry_type).toBe("feat");
      }
    }
  });

  test("?sort=size orders entries by pr_size desc within each version", async ({
    request,
  }) => {
    const res = await request.get("/api/v1/changelog?sort=size");
    expect(res.status()).toBe(200);
    const { versions } = await res.json();
    for (const v of versions) {
      const sizes = v.entries
        .map((e: { pr_size: number | null }) => e.pr_size ?? 0);
      for (let i = 0; i < sizes.length - 1; i++) {
        expect(sizes[i]).toBeGreaterThanOrEqual(sizes[i + 1]);
      }
    }
  });

  test("range param is echoed back in response", async ({ request }) => {
    const res = await request.get(
      "/api/v1/changelog?from=2.11.0&to=2.22.0",
    );
    expect(res.status()).toBe(200);
    const { range } = await res.json();
    expect(range.from).toBe("2.11.0");
    expect(range.to).toBe("2.22.0");
  });
});

test.describe("Changelog page", () => {
  test("GET /changelog returns HTML with changelog viewer", async ({
    request,
  }) => {
    const res = await request.get("/changelog");
    expect(res.status()).toBe(200);
    const ct = res.headers()["content-type"] ?? "";
    expect(ct).toContain("text/html");
    const html = await res.text();
    expect(html).toContain("changelog");
    expect(html).toContain("cl-controls");
  });

  test("/changelog page renders version list", async ({ page }) => {
    await page.goto("/changelog");
    // Wait for the loading state to resolve
    await expect(page.locator(".cl-body")).not.toContainText(
      "Carregando",
      { timeout: 8000 },
    );
    // Should have at least one version block OR an empty-state message
    const versionCount = await page.locator(".cl-version").count();
    const emptyCount = await page.locator(".cl-empty").count();
    expect(versionCount + emptyCount).toBeGreaterThan(0);
  });

  test("type chip filter works client-side", async ({ page }) => {
    await page.goto("/changelog");
    await expect(page.locator(".cl-body")).not.toContainText("Carregando", {
      timeout: 8000,
    });

    // Deactivate all chips first, then activate only 'feat'
    const chips = page.locator(".cl-chip");
    const count = await chips.count();
    for (let i = 0; i < count; i++) {
      const chip = chips.nth(i);
      const isActive = await chip.evaluate((el) =>
        el.classList.contains("active"),
      );
      if (isActive) await chip.click();
    }

    // Activate feat only
    const featChip = page.locator('.cl-chip[data-type="feat"]');
    await featChip.click();

    // Any visible entry pills should be feat (or no entries shown)
    const entries = page.locator(".cl-entry[data-type]");
    const entryCount = await entries.count();
    for (let i = 0; i < entryCount; i++) {
      const entryType = await entries.nth(i).getAttribute("data-type");
      expect(entryType).toBe("feat");
    }
  });

  test("sequential/grouped view toggle works", async ({ page }) => {
    await page.goto("/changelog");
    await expect(page.locator(".cl-body")).not.toContainText("Carregando", {
      timeout: 8000,
    });

    // Default is sequential
    const seqBtn = page.locator("#btn-view-sequential");
    const grpBtn = page.locator("#btn-view-grouped");
    await expect(seqBtn).toHaveClass(/active/);

    // Switch to grouped
    await grpBtn.click();
    await expect(grpBtn).toHaveClass(/active/);
    await expect(seqBtn).not.toHaveClass(/active/);

    // Grouped view should show type-section headers (if data exists)
    const hasVersions =
      (await page.locator(".cl-version").count()) > 0 ||
      (await page.locator(".cl-type-section").count()) > 0 ||
      (await page.locator(".cl-empty").count()) > 0;
    expect(hasVersions).toBe(true);
  });
});

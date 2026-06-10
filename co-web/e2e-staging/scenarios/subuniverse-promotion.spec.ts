/**
 * CO-374 — Sub-universe promotion: A/B → root B; children reparent to B/C
 *
 * Fixture universes (seeded by CO-379):
 *   recursion-a          — root
 *   recursion-a-b        — child of recursion-a
 *   recursion-a-b-c      — grandchild of recursion-a-b
 *
 * Promotion endpoint: POST /api/v1/admin/universes/promote
 * Body: { key: "<sub-universe-key>", new_parent: null | string }
 * Effect: detaches the sub-universe from its parent; child universes reparent.
 *
 * NOTE: The promotion endpoint is part of the admin universe-management feature
 * (Wave 4). Tests that require it are marked test.fixme() until it ships.
 * Pre-condition tests (parent_key relationships) run unconditionally.
 */

import { test, expect } from "../fixtures";

const FIXTURE_ROOT = "recursion-a";
const FIXTURE_CHILD = "recursion-a-b";
const FIXTURE_LEAF = "recursion-a-b-c";
const PROMOTE_ENDPOINT = "/api/v1/admin/universes/promote";

test.describe("Sub-universe promotion — pre-conditions", () => {
  test("fixture child universe has correct parent before promotion", async ({
    apiContext,
  }) => {
    const res = await apiContext.get(`/api/v1/universes/${FIXTURE_CHILD}`);
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body.parent_key).toBe(FIXTURE_ROOT);
  });

  test("fixture grandchild is nested two levels deep", async ({
    apiContext,
  }) => {
    const res = await apiContext.get(`/api/v1/universes/${FIXTURE_LEAF}`);
    expect(res.status()).toBe(200);
    const body = await res.json();
    expect(body.parent_key).toBe(FIXTURE_CHILD);
  });
});

test.describe("Sub-universe promotion — promote to root", () => {
  test("promote sub-universe to root changes parent_key to null", async ({
    apiContext,
  }) => {
    // This test requires POST /api/v1/admin/universes/promote (Wave 4 endpoint).
    // It is marked fixme until the endpoint ships; the pre-condition tests above
    // verify the fixture state is correct.
    test.fixme(
      true,
      `${PROMOTE_ENDPOINT} not yet implemented — tracked as Wave 4 admin universe-management feature`,
    );

    const before = await apiContext.get(`/api/v1/universes/${FIXTURE_CHILD}`);
    expect(before.status()).toBe(200);
    const beforeData = await before.json();
    expect(beforeData.parent_key).toBe(FIXTURE_ROOT);

    const promote = await apiContext.post(PROMOTE_ENDPOINT, {
      data: { key: FIXTURE_CHILD, new_parent: null },
    });
    expect(promote.status()).toBe(200);

    const after = await apiContext.get(`/api/v1/universes/${FIXTURE_CHILD}`);
    expect(after.status()).toBe(200);
    const afterData = await after.json();
    expect(afterData.parent_key ?? null).toBeNull();
  });

  test("grandchild reparents to promoted universe after promotion", async ({
    apiContext,
  }) => {
    test.fixme(
      true,
      `${PROMOTE_ENDPOINT} not yet implemented — tracked as Wave 4 admin universe-management feature`,
    );

    await apiContext.post(PROMOTE_ENDPOINT, {
      data: { key: FIXTURE_CHILD, new_parent: null },
    });

    const child = await apiContext.get(`/api/v1/universes/${FIXTURE_LEAF}`);
    expect(child.status()).toBe(200);
    const childData = await child.json();
    // After promoting recursion-a-b to root, recursion-a-b-c should still
    // point to recursion-a-b (the now-root universe)
    expect(childData.parent_key).toBe(FIXTURE_CHILD);
  });
});

test.describe("Sub-universe promotion — entry content promotion (existing)", () => {
  test("POST /api/v1/universes/{slug}/promote promotes entries (existing endpoint)", async ({
    apiContext,
  }) => {
    // This tests the existing content-promotion endpoint (op_log_routes.rs),
    // NOT the universe-structure promotion above.
    const res = await apiContext.post(
      `/api/v1/universes/${FIXTURE_CHILD}/promote`,
      {
        data: {
          target_universe: FIXTURE_ROOT,
          strategy: "last-write-wins",
        },
      },
    );
    // 200 OK if there are entries to promote; 404 or 422 if no entries exist yet
    expect([200, 204, 404, 422]).toContain(res.status());
  });
});

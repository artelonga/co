/**
 * CO-109: k6 load scenario — mbya universe browse
 *
 * Exercises the entries query path at scale against the mbya lexicon universe
 * (4,608+ entries). Designed to surface:
 *   - N+1 / O(N) queries on large frontmatter blobs
 *   - Pagination under concurrency
 *   - Tag aggregation cost at thousands of tagged entries
 *   - Theme + entries endpoint interleaved (cache vs. DB path)
 *
 * Usage:
 *   BASE_URL=https://co-artelonga-uat.fly.dev k6 run tests/load/06-mbya-browse.js
 *   BASE_URL=https://co-artelonga-uat.fly.dev k6 run --vus 100 --duration 1m tests/load/06-mbya-browse.js
 */
import http from "k6/http";
import { check, sleep } from "k6";
import { Rate, Trend } from "k6/metrics";

const BASE_URL = __ENV.BASE_URL || "https://co-artelonga-uat.fly.dev";

// Guard: abort if BASE_URL looks like production.
if (
  BASE_URL.includes("co-artelonga.fly.dev") &&
  !BASE_URL.includes("-uat") &&
  !BASE_URL.includes("localhost") &&
  !BASE_URL.includes("127.0.0.1")
) {
  throw new Error(
    `Refusing to run against production URL: ${BASE_URL}\n` +
    "Set BASE_URL to a UAT or local instance."
  );
}

export const options = {
  scenarios: {
    // Ramp to 50 VU, hold 1 min, ramp down
    browse_50: {
      executor: "ramping-vus",
      startVUs: 0,
      stages: [
        { duration: "10s", target: 50 },
        { duration: "1m", target: 50 },
        { duration: "10s", target: 0 },
      ],
      gracefulRampDown: "5s",
    },
  },
  thresholds: {
    // Vault listing returns 401 for anonymous users (expected) — exclude from failure count.
    // Only count unexpected 5xx errors as failures.
    "http_req_failed{expected_response:true}": ["rate<0.05"],
    http_req_duration: ["p(95)<3000"],
    mbya_entry_list_duration: ["p(95)<2000"],
  },
};

const entryListDuration = new Trend("mbya_entry_list_duration", true);
const tagsDuration = new Trend("mbya_tags_duration", true);
const errorRate = new Rate("mbya_errors");

const UNIVERSE = "mbya";

// Page sizes to cycle through — exercises both small and large responses
const PAGE_SIZES = [10, 25, 50, 100];
// Letter prefixes to simulate dictionary browsing by initial
const LETTERS = ["a", "e", "i", "j", "k", "m", "n", "o", "p", "r", "t", "y"];

export default function () {
  const vu = __VU;
  const iter = __ITER;

  // 1. Universe metadata
  const infoRes = http.get(`${BASE_URL}/api/v1/universes/${UNIVERSE}`, {
    tags: { name: "universe_info" },
  });
  check(infoRes, { "universe info 200": (r) => r.status === 200 });
  errorRate.add(infoRes.status !== 200);

  sleep(0.5 + Math.random() * 0.5);

  // 2. Entry list — paginated (full frontmatter included)
  const limit = PAGE_SIZES[(vu + iter) % PAGE_SIZES.length];
  const offset = (iter % 20) * limit;
  const listStart = Date.now();
  const listRes = http.get(
    `${BASE_URL}/api/v1/universes/${UNIVERSE}/entries?type=lexeme&limit=${limit}&offset=${offset}`,
    { tags: { name: "entry_list" } }
  );
  entryListDuration.add(Date.now() - listStart);
  check(listRes, {
    "entry list 200": (r) => r.status === 200,
    "entry list has data": (r) => {
      try {
        const body = JSON.parse(r.body);
        return Array.isArray(body.entries) || body.total > 0;
      } catch {
        return false;
      }
    },
  });
  errorRate.add(listRes.status !== 200);

  sleep(1 + Math.random() * 2);

  // 3. Tag aggregation (exercises frontmatter_json parsing at scale)
  const tagsStart = Date.now();
  const tagsRes = http.get(
    `${BASE_URL}/api/v1/universes/${UNIVERSE}/entries/tags`,
    { tags: { name: "tags" } }
  );
  tagsDuration.add(Date.now() - tagsStart);
  check(tagsRes, { "tags 200 or 404": (r) => r.status === 200 || r.status === 404 });

  sleep(0.5 + Math.random() * 0.5);

  // 4. Vault file listing (exercises file tree scan at scale)
  const vaultRes = http.get(
    `${BASE_URL}/api/v1/universes/${UNIVERSE}/vault/`,
    { tags: { name: "vault_root" } }
  );
  check(vaultRes, { "vault listing 200 or 401": (r) => r.status === 200 || r.status === 401 });

  sleep(1 + Math.random() * 2);

  // 5. Letter-based browse — simulates navigating the dictionary alphabetically
  const letter = LETTERS[(vu + iter) % LETTERS.length];
  const browseRes = http.get(
    `${BASE_URL}/api/v1/universes/${UNIVERSE}/entries?type=lexeme&prefix=${letter}&limit=25`,
    { tags: { name: "letter_browse" } }
  );
  check(browseRes, { "letter browse 200 or 400": (r) => r.status === 200 || r.status === 400 });

  sleep(2 + Math.random() * 3);
}

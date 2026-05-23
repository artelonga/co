# Playwright E2E Pattern — CO Platform

## Location

```
co-web/e2e/
├── smoke.spec.ts       # Health + anonymous flow
├── universe.spec.ts    # Universe CRUD
└── auth.spec.ts        # Login flow
```

## Test Structure

```typescript
import { test, expect } from '@playwright/test';

test.describe('Feature', () => {
    test('golden path', async ({ page }) => {
        await page.goto('/');
        await expect(page.locator('h1')).toContainText('CO');
    });

    test('edge case', async ({ page }) => {
        // …
    });
});
```

## Running Tests

```bash
cd co-web

# Against prod (read-only smoke)
BASE_URL=https://co-artelonga.fly.dev npx playwright test e2e/smoke.spec.ts

# Locally
npx playwright test
```

## Common Patterns

### API calls inside tests

```typescript
const resp = await page.request.get('/api/health');
expect(resp.status()).toBe(200);
const body = await resp.json();
expect(body.status).toBe('ok');
```

### Auth-gated pages

```typescript
// Login via UAT endpoint (test env only)
const login = await page.request.post('/api/v1/auth/uat-login', {
    data: { email: 'yuri@uat.local', password: 'uat' }
});
const { session } = await login.json();
await page.context().addCookies([{ name: 'session', value: session, … }]);
```

### Rate-limit safety

Tests run against real environments — add `page.waitForTimeout(200)` between write operations if hitting rate limits (CO-208).

## Assertions to Avoid

- Avoid `page.waitForTimeout()` in the critical path — use `waitForSelector` / `waitForResponse` instead
- Avoid hardcoded text that changes with i18n — use data-testid attributes

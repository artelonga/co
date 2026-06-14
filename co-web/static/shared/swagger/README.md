# Vendored Swagger UI (CO-452)

These files are the **vendored** Swagger UI bundle served at `GET /api/docs`.
They are committed to the repo on purpose — the docs page loads them from
`/shared/swagger/*` (same-origin), not from an external CDN. This keeps the
page CSP-safe and free of third-party runtime dependencies.

| File | Source |
|------|--------|
| `swagger-ui.css` | `swagger-ui-dist@5.32.6` |
| `swagger-ui-bundle.js` | `swagger-ui-dist@5.32.6` |
| `swagger-ui-standalone-preset.js` | `swagger-ui-dist@5.32.6` |
| `*.LICENSE.txt` | upstream license banners (Apache-2.0) |

## Regenerate

```bash
cd co-web
npm install --no-save swagger-ui-dist@5
cp node_modules/swagger-ui-dist/{swagger-ui.css,swagger-ui-bundle.js,swagger-ui-standalone-preset.js} static/shared/swagger/
cp node_modules/swagger-ui-dist/{swagger-ui-bundle.js.LICENSE.txt,swagger-ui-standalone-preset.js.LICENSE.txt} static/shared/swagger/
```

The spec itself is served from `/api/openapi.json` (generated from
`docs/architecture/api-catalog.md` → `co-web/openapi.yaml`), so it is **not**
vendored here — only the UI runtime is.

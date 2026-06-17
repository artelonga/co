## changelog-embed — public, iframeable "releases as sprints" view

New `GET /changelog/embed` — a public (anonymous) page that renders the release
feed as scrum-style sprint cards (version · date · theme · notes), pulling the
already-public `GET /api/v1/changelog/feed`. Styled to match the artelonga
aesthetic (Fraunces/Space Mono, light+dark) so it blends into the embedding page.

Designed to be **iframed by the static artelonga.com.br site** — a live dashboard
inside a static article. Frame headers are handled by a new `frame_headers`
middleware that keeps `X-Frame-Options: DENY` everywhere by default, but serves
`/changelog/embed` with `Content-Security-Policy: frame-ancestors 'self'
https://artelonga.com.br https://*.artelonga.com.br` instead (X-Frame-Options has
no cross-origin allow value, so it's dropped for this one route). 5-minute cache
so new releases surface without redeploying the static site.

Embed with: `<iframe src="https://co.artelonga.com.br/changelog/embed" …>`.

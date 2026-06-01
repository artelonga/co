# DNS Configuration — yuri.artelonga.com.br

## Overview

`yuri.artelonga.com.br` is a personal subdomain that routes to the CO platform and
shows only the `yuri` universe — no multi-universe sidebar, clean URL for sharing
notes/references.

## DNS Record

Add a CNAME at your registrar or DNS provider (e.g. Cloudflare, GoDaddy):

| Type  | Name | Value                    | TTL  |
|-------|------|--------------------------|------|
| CNAME | yuri | co-artelonga.fly.dev.    | 3600 |

For other user subdomains, replace `yuri` with the universe key (must match the
universe key registered in CO, e.g. `alice`, `meu-universo`).

## TLS Certificate (Fly.io)

Fly.io terminates TLS for the app's custom domain. To add certificate coverage for
the subdomain:

```bash
# Add the certificate (Fly issues it via Let's Encrypt)
flyctl certs add yuri.artelonga.com.br -a co-artelonga

# Verify the certificate status
flyctl certs show yuri.artelonga.com.br -a co-artelonga
```

Once the DNS CNAME propagates and the cert is issued, HTTPS works automatically.

## Cookie Domain

Sessions set by CO use the `CO_COOKIE_DOMAIN` environment variable. To allow the
same session cookie to work across `co.artelonga.com.br`, `yuri.artelonga.com.br`,
and any other subdomain, set:

```bash
flyctl secrets set CO_COOKIE_DOMAIN=.artelonga.com.br -a co-artelonga
```

The leading dot makes the cookie valid for all `*.artelonga.com.br` subdomains
(RFC 6265 §5.2.3).

## How Routing Works

1. Browser visits `https://yuri.artelonga.com.br/`
2. Fly.io forwards the request to the CO app with `Host: yuri.artelonga.com.br`
3. CO's `subdomain_routing_middleware` extracts `yuri` from the `Host` header and
   stores it as a `SubdomainUniverse` request extension
4. The SPA-serving handler injects `<script>window.__CO_SUBDOMAIN_UNIVERSE__='yuri';</script>`
   into the response HTML
5. The SPA boots, reads the global, loads the `yuri` universe, and hides the
   multi-universe sidebar via the `co-single-universe-mode` CSS class

## Adding More User Subdomains

To add `alice.artelonga.com.br`:

1. DNS: add `CNAME alice → co-artelonga.fly.dev.`
2. TLS: `flyctl certs add alice.artelonga.com.br -a co-artelonga`
3. No code changes needed — the middleware matches any `*.artelonga.com.br`
   subdomain whose slug matches the universe-key format (`[a-z0-9-]+`)

Reserved prefixes (`co`, `www`) are ignored and fall through to normal routing.

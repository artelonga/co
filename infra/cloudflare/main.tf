terraform {
  required_providers {
    cloudflare = {
      source  = "cloudflare/cloudflare"
      version = "~> 4"
    }
  }
}

variable "cloudflare_api_token" {
  description = "Cloudflare API token with Zone:Edit + DNS:Edit permissions"
  type        = string
  sensitive   = true
}

variable "zone_id" {
  description = "Cloudflare Zone ID for artelonga.com.br"
  type        = string
}

variable "fly_ipv4" {
  description = "Fly.io dedicated IPv4 for co-artelonga (from flyctl ips list)"
  type        = string
}

variable "fly_ipv6" {
  description = "Fly.io IPv6 for co-artelonga"
  type        = string
  default     = ""
}

provider "cloudflare" {
  api_token = var.cloudflare_api_token
}

# --- DNS ---

resource "cloudflare_record" "co_a" {
  zone_id = var.zone_id
  name    = "co"
  type    = "A"
  value   = var.fly_ipv4
  proxied = true
  ttl     = 1 # auto when proxied
}

resource "cloudflare_record" "co_aaaa" {
  count   = var.fly_ipv6 != "" ? 1 : 0
  zone_id = var.zone_id
  name    = "co"
  type    = "AAAA"
  value   = var.fly_ipv6
  proxied = true
  ttl     = 1
}

# --- Cache Rules ---

resource "cloudflare_ruleset" "co_cache" {
  zone_id     = var.zone_id
  name        = "CO cache rules"
  description = "Cache static assets; bypass API and auth responses"
  kind        = "zone"
  phase       = "http_response_headers_transform"

  # Rule 1: never cache auth / Set-Cookie responses (defensive)
  rules {
    action      = "set_cache_settings"
    description = "Bypass: API and auth"
    expression  = "(http.request.uri.path matches \"^/api/\")"
    action_parameters {
      cache = false
    }
    enabled = true
  }
}

resource "cloudflare_cache_rule" "bypass_api" {
  zone_id     = var.zone_id
  name        = "bypass-api"
  description = "Never cache /api/* — all API responses bypass CDN"

  match {
    request {
      uri_path {
        starts_with = "/api/"
      }
    }
  }

  action = "bypass"
}

resource "cloudflare_cache_rule" "static_assets_1y" {
  zone_id     = var.zone_id
  name        = "cache-immutable-1y"
  description = "Cache /_app/immutable/* for 1 year (SvelteKit content-hashed bundles)"

  match {
    request {
      uri_path {
        starts_with = "/_app/immutable/"
      }
    }
  }

  action = "cache"

  action_parameters {
    edge_ttl {
      mode    = "override_origin"
      default = 31536000 # 1 year
    }
    cache_key {
      ignore_query_strings_order = false
    }
  }
}

resource "cloudflare_cache_rule" "theme_css_1h" {
  zone_id     = var.zone_id
  name        = "cache-theme-css-1h"
  description = "Cache theme CSS for 1 hour (short TTL; no content-hash filename)"

  match {
    request {
      uri_path {
        contains = ".css"
      }
    }
  }

  action = "cache"

  action_parameters {
    edge_ttl {
      mode    = "override_origin"
      default = 3600 # 1 hour
    }
  }
}

resource "cloudflare_cache_rule" "images_1d" {
  zone_id     = var.zone_id
  name        = "cache-images-1d"
  description = "Cache images for 1 day"

  match {
    request {
      uri_path {
        matches = "\\.(png|svg|webp|avif|jpg|jpeg|ico)$"
      }
    }
  }

  action = "cache"

  action_parameters {
    edge_ttl {
      mode    = "override_origin"
      default = 86400 # 1 day
    }
  }
}

resource "cloudflare_cache_rule" "html_60s" {
  zone_id     = var.zone_id
  name        = "cache-html-60s"
  description = "Cache HTML for 60 seconds — ISR-like behavior for non-API pages"

  match {
    request {
      uri_path {
        not {
          starts_with = "/api/"
        }
      }
      headers {
        name     = "Accept"
        contains = "text/html"
      }
    }
  }

  action = "cache"

  action_parameters {
    edge_ttl {
      mode    = "override_origin"
      default = 60
    }
    # Never cache if response contains Set-Cookie: session=
    serve_stale {
      disable_stale_while_revalidate = false
    }
  }
}

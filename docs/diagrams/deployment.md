---
title: Co Deployment (C4 Container)
type: doc
tags: [architecture, c4, deployment]
---

# Co Deployment

The platform runs on Fly.io with two environments — UAT for verification and prod for users. Both build from the same `artelonga/co` repo; UAT additionally pulls prod content on reset (CO-82, dormant by default).

```mermaid
C4Container
    title Container view — Co Platform on Fly.io

    Person(user, "End user", "Browses /co universes")
    Person(yuri, "Yuri", "Owner / admin")

    System_Ext(github, "GitHub", "artelonga/co repo")
    System_Ext(fly_registry, "Fly Registry", "Docker images")

    Container_Boundary(uat, "UAT — co-artelonga-uat.fly.dev") {
        Container(uat_web, "co-web (UAT)", "Rust / Axum", "Build: 1.17.0; CO_ENV=uat")
        ContainerDb(uat_db, "co.db (UAT)", "SQLite WAL", "Auto-reset via flag; auto_stop=false")
    }

    Container_Boundary(prod, "Production — co-artelonga.fly.dev") {
        Container(prod_web, "co-web (prod)", "Rust / Axum", "Build: 1.17.0; CO_ENV unset")
        ContainerDb(prod_db, "co.db (prod)", "SQLite WAL", "Persistent volume; auto_stop=true")
    }

    Rel(yuri, github, "Push commits")
    Rel(github, fly_registry, "Build + push image", "flyctl deploy")
    Rel(fly_registry, uat_web, "Pull image")
    Rel(fly_registry, prod_web, "Pull image")
    Rel(user, prod_web, "HTTPS / WebSocket")
    Rel(yuri, uat_web, "HTTPS / verification")
    Rel(uat_web, prod_web, "Vault REST (CO-82, dormant)", "GET /api/v1/universes/* read-only")
```

## Notes

- The UAT → prod arrow is dormant until `UAT_MIRROR_PROD=true` and `UAT_PROD_TOKEN` are set as Fly secrets on `co-artelonga-uat`.
- Both environments deploy from the same Cargo.toml workspace version; `Cargo.toml` bumps land in one PR with the feature.
- Dev tooling (`co-auto`) lives in `dev/co-auto/` and is NOT part of either container — it's a developer's local binary.

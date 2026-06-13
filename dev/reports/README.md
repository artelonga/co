# Pipeline reports (CO-88)

`co-pipeline run` writes `co-pipeline-report-<date>.yaml` here. The report is
deterministic except for the wall-clock `generated_at`, so CI can diff two runs
and surface compression-ratio / transfer-time regressions.

Generated reports are git-ignored (see `.gitignore`); CI keeps them as build
artifacts and compares against the previous run.

```bash
# Local filesystem matrix (Path A) over the real corpora:
cargo run -p co-pipeline -- run --corpus-root ~/projects --paths local

# Full network matrix against UAT:
cargo run -p co-pipeline -- run \
  --corpus-root ~/projects \
  --paths local,uat \
  --uat-base https://co-artelonga-uat.fly.dev \
  --token "$CO_PIPELINE_TOKEN"
```

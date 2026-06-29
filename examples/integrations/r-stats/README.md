# r-stats — R as a deterministic CO tool

**What it demonstrates.** Any R script becomes a first-class CO tool through a
**manifest entry alone — zero binary recompile**. This is the R / RStudio vision
for CO-503: the statistical computing ecosystem plugs into the canonical tool
contract the same way a shell one-liner does.

This sample is the `deterministic` / subprocess shape: CO's `SubprocessInvoker`
spawns the `command`, writes the JSON args to the child's **stdin**, and parses
the child's **stdout** as JSON (see `co/src/canon_tool.rs`). `summary.R` honours
exactly that contract using **base R only** (no packages), so it is reviewable
and runnable wherever R is installed.

## AS-IS vs TO-BE

| | |
|---|---|
| **AS-IS** | Stats live outside CO. You copy numbers into RStudio, run `summary()` by hand, and paste results back. An agent can't reach R at all. |
| **TO-BE** | `r-stats.yaml` registers the script as a tool. An agent lists it next to native tools and calls it like any other — R runs out-of-process, output flows back as JSON. The stateful **RStudio-web REPL** (interactive sessions, not one-shot scripts) is the separate follow-up **CO-524**. |

R is declared under `dependencies:` in the manifest so a reviewer understands the
runtime requirement **without needing R installed to read the sample**.

## How an agent calls it

The agent sees the `input_schema` and supplies matching JSON; the tool returns a
JSON object.

Input (inline values):
```json
{ "values": [10, 12, 23, 23, 16, 23, 21, 16] }
```
Output:
```json
{ "n":8, "mean":18, "median":18.5, "sd":4.956, "min":10, "max":23,
  "q25":15, "q50":18.5, "q75":23 }
```

Input (CSV path) — first column is read, header auto-skipped:
```json
{ "csv_path": "examples/integrations/r-stats/sample.csv" }
```

## Try it (requires R / Rscript)

```bash
# inline values
echo '{"values":[10,12,23,23,16,23,21,16]}' \
  | Rscript examples/integrations/r-stats/summary.R

# from CSV
echo '{"csv_path":"examples/integrations/r-stats/sample.csv"}' \
  | Rscript examples/integrations/r-stats/summary.R
```

If R is not installed, read `summary.R` — the stdin→stdout JSON contract is the
whole point; the language behind it is interchangeable.

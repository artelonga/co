#!/usr/bin/env Rscript
# summary.R — CO-503 deterministic subprocess tool (the R / RStudio vision).
#
# Contract (matches co/src/canon_tool.rs::SubprocessInvoker):
#   * Reads ONE JSON object from stdin: {"values":[..]} OR {"csv_path":"..."}.
#   * Writes ONE JSON object to stdout: the summary stats.
#   * Exits non-zero with a message on stderr on any error.
#
# Uses only base R (no packages) so it runs anywhere R is installed.

# --- tiny zero-dependency JSON reader/writer -------------------------------
# base R has no JSON; for this sample we parse the narrow shapes we accept and
# emit JSON by hand. A real deployment would `library(jsonlite)` and declare it
# under `dependencies:` in the manifest.

read_stdin <- function() {
  con <- file("stdin", "r")
  on.exit(close(con))
  paste(readLines(con, warn = FALSE), collapse = "")
}

# Extract a JSON number array for "values": [...]
parse_values <- function(txt) {
  m <- regmatches(txt, regexpr('"values"\\s*:\\s*\\[[^]]*\\]', txt))
  if (length(m) == 0) return(NULL)
  inside <- sub('.*\\[(.*)\\].*', '\\1', m)
  inside <- trimws(inside)
  if (nchar(inside) == 0) return(numeric(0))
  as.numeric(strsplit(inside, "\\s*,\\s*")[[1]])
}

# Extract a JSON string for "csv_path": "..."
parse_csv_path <- function(txt) {
  m <- regmatches(txt, regexpr('"csv_path"\\s*:\\s*"[^"]*"', txt))
  if (length(m) == 0) return(NULL)
  sub('.*"csv_path"\\s*:\\s*"([^"]*)".*', '\\1', m)
}

fail <- function(msg) {
  cat(msg, "\n", file = stderr())
  quit(status = 1)
}

# --- gather input ----------------------------------------------------------
raw <- read_stdin()
vals <- parse_values(raw)

if (is.null(vals)) {
  csv_path <- parse_csv_path(raw)
  if (is.null(csv_path)) fail("provide either `values` (number[]) or `csv_path` (string)")
  if (!file.exists(csv_path)) fail(paste0("csv_path not found: ", csv_path))
  df <- utils::read.csv(csv_path, header = NA)  # header auto-detected
  vals <- suppressWarnings(as.numeric(df[[1]]))
  vals <- vals[!is.na(vals)]
}

if (length(vals) == 0) fail("no numeric values to summarize")

# --- compute (base R) ------------------------------------------------------
q <- stats::quantile(vals, probs = c(0.25, 0.5, 0.75), names = FALSE)
res <- list(
  n      = length(vals),
  mean   = mean(vals),
  median = stats::median(vals),
  sd     = if (length(vals) > 1) stats::sd(vals) else 0,
  min    = min(vals),
  max    = max(vals),
  q25    = q[1],
  q50    = q[2],
  q75    = q[3]
)

# --- emit JSON -------------------------------------------------------------
num <- function(x) formatC(x, format = "g", digits = 10)
pairs <- vapply(names(res), function(k) {
  v <- res[[k]]
  if (k == "n") paste0('"', k, '":', v)
  else paste0('"', k, '":', num(v))
}, character(1))
cat("{", paste(pairs, collapse = ","), "}\n", sep = "")

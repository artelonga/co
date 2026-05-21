#' Connect to a CO universe's data.db via DuckDB
#'
#' Opens a read-only DuckDB connection to a CO universe's per-universe SQLite
#' database. Tries the local data directory first; falls back to downloading a
#' snapshot from the CO API.
#'
#' @param slug Universe key (e.g. \code{"template"}).
#' @param base_url Base URL of the CO server (e.g. \code{"https://co.example.com"}).
#'   Required when \code{data_dir} is \code{NULL} or does not contain the universe.
#' @param api_token Bearer token for authenticated API access.
#' @param data_dir Root data directory. When set, resolves
#'   \code{data_dir/universes/*/*/slug/data.db} via glob.
#'
#' @return A \code{DBIConnection} opened read-only via duckdb.
#' @export
co_connect <- function(slug, base_url = NULL, api_token = NULL, data_dir = NULL) {
  db_path <- NULL

  if (!is.null(data_dir)) {
    pattern <- file.path(data_dir, "universes", "*", "*", slug, "data.db")
    matches <- Sys.glob(pattern)
    if (length(matches) > 0) {
      db_path <- matches[[1]]
    }
  }

  if (is.null(db_path)) {
    if (is.null(base_url)) {
      stop(sprintf(
        "Universe '%s' not found under data_dir=%s and no base_url was given.",
        slug, deparse(data_dir)
      ))
    }
    db_path <- .download_snapshot(slug, base_url = base_url, api_token = api_token)
  }

  DBI::dbConnect(duckdb::duckdb(), dbdir = db_path, read_only = TRUE)
}

.download_snapshot <- function(slug, base_url, api_token) {
  url <- paste0(sub("/$", "", base_url), "/api/v1/universes/", slug, "/db-snapshot")
  headers <- c("Content-Type" = "application/json")
  if (!is.null(api_token)) {
    headers <- c(headers, "Authorization" = paste("Bearer", api_token))
  }
  resp <- httr::POST(url, httr::add_headers(.headers = headers), body = "{}")
  if (httr::http_error(resp)) {
    stop(sprintf(
      "Failed to download snapshot for universe '%s' from %s: HTTP %d",
      slug, url, httr::status_code(resp)
    ))
  }
  tmp <- tempfile(fileext = ".db")
  writeBin(httr::content(resp, "raw"), tmp)
  tmp
}

//! CO-74 / CO-325: Query DSL — compiled to parameterized SQLite.
//!
//! # Syntax
//!
//! Two forms are accepted:
//!
//! ## FROM … WHERE … (original, CO-74)
//!
//! ```text
//! FROM <type> [WHERE <cond> [AND <cond>]*] [LIMIT <n>]
//!
//! <cond> ::= <field> = <quoted>              -- exact frontmatter match
//!          | <field> LIKE <quoted>            -- LIKE frontmatter match
//!          | <field> INCLUDES <quoted>        -- entry_relations join
//!          | <field> IS NOT NULL              -- field presence check
//! ```
//!
//! ## Shorthand (CO-325)
//!
//! ```text
//! [type:<type_or_category>] [AND key:value]*
//!
//! Special keys:
//!   type:     — content type or category (music → song + album)
//!   before:   — created_at < value
//!   after:    — created_at > value
//!   All other keys become  field = "value"  conditions.
//! ```
//!
//! # Examples
//!
//! ```text
//! FROM evento WHERE attendees INCLUDES "yuri"
//! type:music AND author:"Yuri"
//! type:notas AND caderno_id:"black-2024"
//! before:2026-05-01 AND after:2025-01-01
//! FROM song WHERE references.youtube IS NOT NULL
//! ```
//!
//! # Safety
//!
//! All user-supplied values are bound via SQLite parameters (`?N`).  Field
//! names are validated: only alphanumeric + `_` + `-` + `.` are permitted
//! before being interpolated into `json_extract` paths.
//!
//! # Scale
//!
//! Results are capped at 1 000 rows.  An explicit `LIMIT` is clamped.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Hard cap on rows returned per query.
pub const MAX_RESULT_ROWS: usize = 1000;

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum QueryError {
    Parse(String),
    TooComplex(String),
    UnsafeFieldName(String),
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::Parse(msg) => write!(f, "parse error: {msg}"),
            QueryError::TooComplex(msg) => write!(f, "query too complex: {msg}"),
            QueryError::UnsafeFieldName(name) => write!(
                f,
                "unsafe field name '{name}': must contain only alphanumeric chars, '-', '_', or '.'"
            ),
        }
    }
}

impl std::error::Error for QueryError {}

// ---------------------------------------------------------------------------
// AST
// ---------------------------------------------------------------------------

/// Parsed representation of a DSL query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DslQuery {
    /// The type or category as written by the user (e.g. `"music"`).
    pub from_type: String,
    /// Expanded type list after category resolution.
    ///
    /// Populated by [`resolve`]; supersedes `from_type` in [`compile`].
    /// Empty means use `from_type` directly (or no type filter if `"*"`).
    #[serde(default)]
    pub from_types: Vec<String>,
    pub filters: Vec<DslFilter>,
    pub limit: usize,
}

/// A single filter condition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DslFilter {
    /// `field = "value"` — exact match via `json_extract`.
    FieldEq { field: String, value: String },
    /// `field LIKE "pattern"` — LIKE match via `json_extract`.
    FieldLike { field: String, pattern: String },
    /// `field INCLUDES "target"` — join on `entry_relations` (to_path LIKE %target%).
    RelationIncludes { field: String, target: String },
    /// `field IS NOT NULL` — check that a frontmatter field is present.
    ///
    /// For column fields (`created_at`, `updated_at`) uses the column directly.
    FieldNotNull { field: String },
    /// `before:<date>` — `created_at < value` (or json_extract for other fields).
    DateBefore { field: String, value: String },
    /// `after:<date>` — `created_at > value` (or json_extract for other fields).
    DateAfter { field: String, value: String },
}

// ---------------------------------------------------------------------------
// Category resolution
// ---------------------------------------------------------------------------

/// Default type-to-category mapping (derived from `work/co/schema/*.yaml`).
///
/// Returns a map of `category → [subtypes]` used by [`resolve`].
pub fn default_type_categories() -> HashMap<String, Vec<String>> {
    let mut m = HashMap::new();
    m.insert(
        "music".to_string(),
        vec!["song".to_string(), "album".to_string()],
    );
    m.insert(
        "writing".to_string(),
        vec!["poem".to_string(), "essay".to_string()],
    );
    m.insert("media".to_string(), vec!["video".to_string()]);
    m.insert(
        "reference".to_string(),
        vec!["url".to_string(), "quote".to_string(), "notas".to_string()],
    );
    m
}

/// Resolve a category name in `query.from_type` to its constituent types.
///
/// If `from_type` is a known category, `from_types` is populated with the
/// expanded type list.  If it is already a specific type (or `"*"`), the
/// query is returned unchanged.
pub fn resolve(mut query: DslQuery, categories: &HashMap<String, Vec<String>>) -> DslQuery {
    if let Some(types) = categories.get(&query.from_type) {
        query.from_types = types.clone();
    }
    query
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse a DSL query string into a [`DslQuery`] AST.
///
/// Accepts both the `FROM … WHERE …` (CO-74) syntax and the `type:X AND
/// field:value` shorthand (CO-325).  The FROM form is tried first; if the
/// input does not start with `FROM` (case-insensitive), the shorthand parser
/// is used.
pub fn parse(input: &str) -> Result<DslQuery, QueryError> {
    let trimmed = input.trim();
    if trimmed
        .split_whitespace()
        .next()
        .map(|w| w.eq_ignore_ascii_case("FROM"))
        .unwrap_or(false)
    {
        let tokens = tokenize(trimmed)?;
        parse_tokens(&tokens)
    } else {
        parse_shorthand(trimmed)
    }
}

/// Parse the shorthand `type:X AND key:value` syntax.
///
/// Special keys: `type` (sets from_type), `before` / `after` (date range).
/// All other keys become `FieldEq` conditions.
pub fn parse_shorthand(input: &str) -> Result<DslQuery, QueryError> {
    let tokens = tokenize(input)?;
    if tokens.is_empty() {
        return Err(QueryError::Parse("empty query".into()));
    }

    let mut from_type = "*".to_string();
    let mut filters: Vec<DslFilter> = Vec::new();
    let mut pos = 0;

    while pos < tokens.len() {
        // Skip AND keywords between terms
        if let Token::Word(w) = &tokens[pos]
            && w.eq_ignore_ascii_case("AND")
        {
            pos += 1;
            continue;
        }

        match &tokens[pos] {
            Token::Word(w) => {
                if let Some(colon) = w.find(':') {
                    let key = w[..colon].to_string();
                    let inline_val = w[colon + 1..].to_string();
                    pos += 1;

                    let value = if !inline_val.is_empty() {
                        inline_val
                    } else {
                        // value is the next token
                        match tokens.get(pos) {
                            Some(Token::Quoted(s)) => {
                                let s = s.clone();
                                pos += 1;
                                s
                            }
                            Some(Token::Word(s)) => {
                                let s = s.clone();
                                pos += 1;
                                s
                            }
                            Some(Token::Number(n)) => {
                                let s = n.to_string();
                                pos += 1;
                                s
                            }
                            None => {
                                return Err(QueryError::Parse(format!(
                                    "expected value after '{key}:'"
                                )));
                            }
                        }
                    };

                    match key.as_str() {
                        "type" => from_type = value,
                        "before" => filters.push(DslFilter::DateBefore {
                            field: "created_at".to_string(),
                            value,
                        }),
                        "after" => filters.push(DslFilter::DateAfter {
                            field: "created_at".to_string(),
                            value,
                        }),
                        _ => {
                            validate_field_name(&key)?;
                            filters.push(DslFilter::FieldEq { field: key, value });
                        }
                    }
                } else {
                    return Err(QueryError::Parse(format!(
                        "expected key:value pair, got: '{w}'"
                    )));
                }
            }
            other => {
                return Err(QueryError::Parse(format!("unexpected token: {other}")));
            }
        }
    }

    if filters.len() > 10 {
        return Err(QueryError::TooComplex(
            "max 10 filter conditions per query".into(),
        ));
    }

    Ok(DslQuery {
        from_type,
        from_types: vec![],
        filters,
        limit: MAX_RESULT_ROWS,
    })
}

#[derive(Debug, PartialEq)]
enum Token {
    Word(String),
    Quoted(String),
    Number(usize),
}

impl std::fmt::Display for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Token::Word(w) => write!(f, "{w}"),
            Token::Quoted(s) => write!(f, "\"{s}\""),
            Token::Number(n) => write!(f, "{n}"),
        }
    }
}

fn tokenize(input: &str) -> Result<Vec<Token>, QueryError> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(&c) = chars.peek() {
        match c {
            ' ' | '\t' | '\n' | '\r' => {
                chars.next();
            }
            '"' | '\'' => {
                let quote = c;
                chars.next();
                let mut s = String::new();
                let mut closed = false;
                for ch in chars.by_ref() {
                    if ch == quote {
                        closed = true;
                        break;
                    }
                    s.push(ch);
                }
                if !closed {
                    return Err(QueryError::Parse("unterminated quoted string".into()));
                }
                tokens.push(Token::Quoted(s));
            }
            '0'..='9' => {
                let mut s = String::new();
                while let Some(&d) = chars.peek() {
                    if d.is_ascii_digit() {
                        s.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
                let n: usize = s
                    .parse()
                    .map_err(|_| QueryError::Parse(format!("invalid number: {s}")))?;
                tokens.push(Token::Number(n));
            }
            _ => {
                let mut s = String::new();
                while let Some(&ch) = chars.peek() {
                    if ch == ' '
                        || ch == '\t'
                        || ch == '\n'
                        || ch == '\r'
                        || ch == '"'
                        || ch == '\''
                    {
                        break;
                    }
                    s.push(ch);
                    chars.next();
                }
                tokens.push(Token::Word(s));
            }
        }
    }

    Ok(tokens)
}

fn parse_tokens(tokens: &[Token]) -> Result<DslQuery, QueryError> {
    let mut pos = 0;

    expect_keyword(tokens, &mut pos, "FROM")?;
    let from_type = take_word(tokens, &mut pos, "content type after FROM")?;

    let mut filters: Vec<DslFilter> = Vec::new();
    let mut limit = MAX_RESULT_ROWS;

    while pos < tokens.len() {
        match &tokens[pos] {
            Token::Word(w) if w.eq_ignore_ascii_case("WHERE") => {
                pos += 1;
                filters.push(parse_condition(tokens, &mut pos)?);
                loop {
                    if let Some(Token::Word(w)) = tokens.get(pos)
                        && w.eq_ignore_ascii_case("AND")
                    {
                        pos += 1;
                        filters.push(parse_condition(tokens, &mut pos)?);
                        continue;
                    }
                    break;
                }
            }
            Token::Word(w) if w.eq_ignore_ascii_case("LIMIT") => {
                pos += 1;
                match tokens.get(pos) {
                    Some(Token::Number(n)) => {
                        limit = (*n).min(MAX_RESULT_ROWS);
                        pos += 1;
                    }
                    other => {
                        return Err(QueryError::Parse(format!(
                            "expected number after LIMIT, got: {other:?}"
                        )));
                    }
                }
            }
            Token::Word(w) => {
                return Err(QueryError::Parse(format!("unexpected token: '{w}'")));
            }
            other => {
                return Err(QueryError::Parse(format!("unexpected token: {other}")));
            }
        }
    }

    if filters.len() > 10 {
        return Err(QueryError::TooComplex(
            "max 10 filter conditions per query".into(),
        ));
    }

    Ok(DslQuery {
        from_type,
        from_types: vec![],
        filters,
        limit,
    })
}

fn parse_condition(tokens: &[Token], pos: &mut usize) -> Result<DslFilter, QueryError> {
    let field = take_word(tokens, pos, "field name")?;
    validate_field_name(&field)?;

    let op = take_word(tokens, pos, "operator (=, LIKE, INCLUDES, IS)")?;

    match op.to_ascii_uppercase().as_str() {
        "=" => {
            let value = take_value(tokens, pos, &op)?;
            Ok(DslFilter::FieldEq { field, value })
        }
        "LIKE" => {
            let pattern = take_value(tokens, pos, &op)?;
            Ok(DslFilter::FieldLike { field, pattern })
        }
        "INCLUDES" => {
            let target = take_value(tokens, pos, &op)?;
            Ok(DslFilter::RelationIncludes { field, target })
        }
        "IS" => {
            let not_kw = take_word(tokens, pos, "NOT")?;
            if !not_kw.eq_ignore_ascii_case("NOT") {
                return Err(QueryError::Parse(format!(
                    "expected NOT after IS, got: '{not_kw}'"
                )));
            }
            let null_kw = take_word(tokens, pos, "NULL")?;
            if !null_kw.eq_ignore_ascii_case("NULL") {
                return Err(QueryError::Parse(format!(
                    "expected NULL after IS NOT, got: '{null_kw}'"
                )));
            }
            Ok(DslFilter::FieldNotNull { field })
        }
        other => Err(QueryError::Parse(format!(
            "unknown operator '{other}', expected =, LIKE, INCLUDES, or IS"
        ))),
    }
}

fn take_value(tokens: &[Token], pos: &mut usize, op: &str) -> Result<String, QueryError> {
    match tokens.get(*pos) {
        Some(Token::Quoted(s)) => {
            let s = s.clone();
            *pos += 1;
            Ok(s)
        }
        Some(Token::Word(s)) => {
            let s = s.clone();
            *pos += 1;
            Ok(s)
        }
        Some(Token::Number(n)) => {
            let s = n.to_string();
            *pos += 1;
            Ok(s)
        }
        None => Err(QueryError::Parse(format!(
            "expected value after operator '{op}', got end of input"
        ))),
    }
}

fn expect_keyword(tokens: &[Token], pos: &mut usize, keyword: &str) -> Result<(), QueryError> {
    match tokens.get(*pos) {
        Some(Token::Word(w)) if w.eq_ignore_ascii_case(keyword) => {
            *pos += 1;
            Ok(())
        }
        other => Err(QueryError::Parse(format!(
            "expected '{keyword}', got: {other:?}"
        ))),
    }
}

fn take_word(tokens: &[Token], pos: &mut usize, context: &str) -> Result<String, QueryError> {
    match tokens.get(*pos) {
        Some(Token::Word(w)) => {
            let w = w.clone();
            *pos += 1;
            Ok(w)
        }
        other => Err(QueryError::Parse(format!(
            "expected {context}, got: {other:?}"
        ))),
    }
}

/// Field names are interpolated into `json_extract` paths.
/// Allowed: alphanumeric, `_`, `-`, `.` (dot enables nested paths like
/// `references.youtube` → `$.references.youtube`).
fn validate_field_name(field: &str) -> Result<(), QueryError> {
    if field.is_empty()
        || !field
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
    {
        Err(QueryError::UnsafeFieldName(field.to_string()))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Column-vs-JSON field handling
// ---------------------------------------------------------------------------

/// Fields that map to real columns (not JSON extraction).
const COLUMN_FIELDS: &[&str] = &["created_at", "updated_at", "entry_type", "title", "path"];

fn field_expr(field: &str) -> String {
    if COLUMN_FIELDS.contains(&field) {
        format!("e.{field}")
    } else {
        format!("json_extract(e.frontmatter_json, '$.{field}')")
    }
}

// ---------------------------------------------------------------------------
// SQL compiler
// ---------------------------------------------------------------------------

/// Compile a [`DslQuery`] to a parameterized SQLite `(sql, params)` pair.
///
/// `?1` is always the `universe_key`.  The returned `params` vec is aligned
/// with the `?N` placeholders in the SQL string.
///
/// Category expansion: if `query.from_types` is non-empty (populated by
/// [`resolve`]), the SQL uses `IN (…)` instead of a single `=` check.
/// If `query.from_type` is `"*"` or empty, no type filter is applied.
pub fn compile(query: &DslQuery, universe_key: &str) -> (String, Vec<String>) {
    let mut params: Vec<String> = vec![universe_key.to_string()];
    let mut conditions: Vec<String> = vec!["e.universe_key = ?1".to_string()];

    // Type filter — from_types (category expansion) takes precedence.
    if !query.from_types.is_empty() {
        let placeholders: Vec<String> = query
            .from_types
            .iter()
            .map(|t| {
                let idx = push_param(&mut params, t);
                format!("?{idx}")
            })
            .collect();
        conditions.push(format!("e.entry_type IN ({})", placeholders.join(", ")));
    } else if !query.from_type.is_empty() && query.from_type != "*" {
        let type_idx = push_param(&mut params, &query.from_type);
        conditions.push(format!("e.entry_type = ?{type_idx}"));
    }
    // from_type == "*" or empty → no type filter

    let needs_distinct = query
        .filters
        .iter()
        .any(|f| matches!(f, DslFilter::RelationIncludes { .. }));

    let mut join_idx = 0usize;
    let mut joins: Vec<String> = Vec::new();

    for filter in &query.filters {
        match filter {
            DslFilter::FieldEq { field, value } => {
                let idx = push_param(&mut params, value);
                conditions.push(format!("{} = ?{idx}", field_expr(field)));
            }
            DslFilter::FieldLike { field, pattern } => {
                let idx = push_param(&mut params, pattern);
                conditions.push(format!("{} LIKE ?{idx}", field_expr(field)));
            }
            DslFilter::RelationIncludes { field, target } => {
                let alias = format!("r{join_idx}");
                join_idx += 1;
                joins.push(format!(
                    "JOIN entry_relations {alias} \
                     ON {alias}.from_path = e.path AND {alias}.universe_key = e.universe_key"
                ));
                let rel_idx = push_param(&mut params, field);
                let target_pattern = format!("%{target}%");
                let target_idx = push_param(&mut params, &target_pattern);
                conditions.push(format!(
                    "{alias}.relation_type = ?{rel_idx} AND {alias}.to_path LIKE ?{target_idx}"
                ));
            }
            DslFilter::FieldNotNull { field } => {
                conditions.push(format!("{} IS NOT NULL", field_expr(field)));
            }
            DslFilter::DateBefore { field, value } => {
                let idx = push_param(&mut params, value);
                conditions.push(format!("{} < ?{idx}", field_expr(field)));
            }
            DslFilter::DateAfter { field, value } => {
                let idx = push_param(&mut params, value);
                conditions.push(format!("{} > ?{idx}", field_expr(field)));
            }
        }
    }

    let join_sql = if joins.is_empty() {
        String::new()
    } else {
        format!(" {}", joins.join(" "))
    };

    let where_sql = conditions.join(" AND ");
    let select_prefix = if needs_distinct { "DISTINCT " } else { "" };
    let limit = query.limit.min(MAX_RESULT_ROWS);

    let sql = format!(
        "SELECT {select_prefix}e.path, e.universe_key, e.entry_type, e.title, \
         e.frontmatter_json, e.body, e.body_hash, e.created_at, e.updated_at \
         FROM entries e{join_sql} WHERE {where_sql} LIMIT {limit}"
    );

    (sql, params)
}

fn push_param(params: &mut Vec<String>, value: &str) -> usize {
    params.push(value.to_string());
    params.len()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- parse (FROM syntax) ----

    #[test]
    fn test_parse_minimal_from() {
        let q = parse("FROM evento").unwrap();
        assert_eq!(q.from_type, "evento");
        assert!(q.filters.is_empty());
        assert_eq!(q.limit, MAX_RESULT_ROWS);
    }

    #[test]
    fn test_parse_field_eq() {
        let q = parse(r#"FROM tarefa WHERE status = "todo""#).unwrap();
        assert_eq!(q.from_type, "tarefa");
        assert_eq!(
            q.filters,
            vec![DslFilter::FieldEq {
                field: "status".to_string(),
                value: "todo".to_string(),
            }]
        );
    }

    #[test]
    fn test_parse_relation_includes() {
        let q = parse(r#"FROM evento WHERE attendees INCLUDES "yuri""#).unwrap();
        assert_eq!(
            q.filters,
            vec![DslFilter::RelationIncludes {
                field: "attendees".to_string(),
                target: "yuri".to_string(),
            }]
        );
    }

    #[test]
    fn test_parse_multiple_conditions() {
        let q = parse(r#"FROM evento WHERE attendees INCLUDES "yuri" AND status = "confirmed""#)
            .unwrap();
        assert_eq!(q.filters.len(), 2);
    }

    #[test]
    fn test_parse_limit_respected() {
        let q = parse("FROM evento LIMIT 50").unwrap();
        assert_eq!(q.limit, 50);
    }

    #[test]
    fn test_parse_limit_clamped_at_max() {
        let q = parse("FROM evento LIMIT 99999").unwrap();
        assert_eq!(q.limit, MAX_RESULT_ROWS);
    }

    #[test]
    fn test_parse_missing_from_keyword() {
        // Without FROM: falls through to shorthand parser; "evento" alone
        // has no colon so shorthand fails.
        assert!(parse("evento WHERE status = \"todo\"").is_err());
    }

    #[test]
    fn test_parse_unterminated_string() {
        assert!(parse(r#"FROM evento WHERE status = "open"#).is_err());
    }

    #[test]
    fn test_parse_unknown_operator() {
        assert!(parse(r#"FROM evento WHERE status != "open""#).is_err());
    }

    #[test]
    fn test_parse_too_many_conditions() {
        let conds = (0..11)
            .map(|i| format!(r#"f{i} = "v{i}""#))
            .collect::<Vec<_>>()
            .join(" AND ");
        let input = format!("FROM t WHERE {conds}");
        assert!(
            parse(&input).is_err(),
            "more than 10 conditions should fail"
        );
    }

    #[test]
    fn test_parse_unsafe_field_name_rejected() {
        assert!(
            parse(r#"FROM evento WHERE a;b = "v""#).is_err(),
            "field name with semicolon must be rejected"
        );
    }

    // ---- IS NOT NULL ----

    #[test]
    fn test_parse_is_not_null() {
        let q = parse("FROM song WHERE references.youtube IS NOT NULL").unwrap();
        assert_eq!(q.from_type, "song");
        assert_eq!(
            q.filters,
            vec![DslFilter::FieldNotNull {
                field: "references.youtube".to_string(),
            }]
        );
    }

    #[test]
    fn test_dotted_field_name_allowed() {
        let q = parse(r#"FROM song WHERE references.spotify = "https://open.spotify.com/foo""#)
            .unwrap();
        assert_eq!(
            q.filters,
            vec![DslFilter::FieldEq {
                field: "references.spotify".to_string(),
                value: "https://open.spotify.com/foo".to_string(),
            }]
        );
    }

    // ---- shorthand parser ----

    #[test]
    fn test_shorthand_type_only() {
        let q = parse("type:music").unwrap();
        assert_eq!(q.from_type, "music");
        assert!(q.filters.is_empty());
    }

    #[test]
    fn test_shorthand_type_with_author() {
        let q = parse(r#"type:song AND author:"Yuri""#).unwrap();
        assert_eq!(q.from_type, "song");
        assert_eq!(
            q.filters,
            vec![DslFilter::FieldEq {
                field: "author".to_string(),
                value: "Yuri".to_string(),
            }]
        );
    }

    #[test]
    fn test_shorthand_caderno_id() {
        let q = parse(r#"type:notas AND caderno_id:"black-2024""#).unwrap();
        assert_eq!(q.from_type, "notas");
        assert_eq!(
            q.filters,
            vec![DslFilter::FieldEq {
                field: "caderno_id".to_string(),
                value: "black-2024".to_string(),
            }]
        );
    }

    #[test]
    fn test_shorthand_before_after() {
        let q = parse("type:notas AND before:2026-05-01 AND after:2025-01-01").unwrap();
        assert_eq!(q.from_type, "notas");
        assert_eq!(q.filters.len(), 2);
        assert!(matches!(
            &q.filters[0],
            DslFilter::DateBefore { field, value }
            if field == "created_at" && value == "2026-05-01"
        ));
        assert!(matches!(
            &q.filters[1],
            DslFilter::DateAfter { field, value }
            if field == "created_at" && value == "2025-01-01"
        ));
    }

    #[test]
    fn test_shorthand_no_type_is_wildcard() {
        let q = parse(r#"caderno_id:"black-2024""#).unwrap();
        assert_eq!(q.from_type, "*");
        assert_eq!(q.filters.len(), 1);
    }

    // ---- resolve (category expansion) ----

    #[test]
    fn test_resolve_category_expands_types() {
        let cats = default_type_categories();
        let q = parse("type:music").unwrap();
        let q = resolve(q, &cats);
        assert!(q.from_types.contains(&"song".to_string()));
        assert!(q.from_types.contains(&"album".to_string()));
    }

    #[test]
    fn test_resolve_specific_type_unchanged() {
        let cats = default_type_categories();
        let q = parse("FROM song").unwrap();
        let q = resolve(q, &cats);
        assert!(q.from_types.is_empty(), "specific type should not expand");
    }

    // ---- compile ----

    #[test]
    fn test_compile_minimal() {
        let q = DslQuery {
            from_type: "evento".to_string(),
            from_types: vec![],
            filters: vec![],
            limit: 100,
        };
        let (sql, params) = compile(&q, "myuniverse");
        assert!(sql.contains("FROM entries e"));
        assert!(sql.contains("e.universe_key = ?1"));
        assert!(sql.contains("e.entry_type = ?2"));
        assert_eq!(params[0], "myuniverse");
        assert_eq!(params[1], "evento");
    }

    #[test]
    fn test_compile_field_eq_adds_json_extract() {
        let q = parse(r#"FROM tarefa WHERE status = "todo""#).unwrap();
        let (sql, params) = compile(&q, "u1");
        assert!(sql.contains("json_extract(e.frontmatter_json, '$.status')"));
        assert!(params.contains(&"todo".to_string()));
    }

    #[test]
    fn test_compile_includes_adds_join() {
        let q = parse(r#"FROM evento WHERE attendees INCLUDES "yuri""#).unwrap();
        let (sql, params) = compile(&q, "u1");
        assert!(
            sql.contains("JOIN entry_relations"),
            "INCLUDES must join entry_relations"
        );
        assert!(
            sql.contains("DISTINCT"),
            "INCLUDES must use DISTINCT to avoid duplicates"
        );
        assert!(params.iter().any(|p| p == "%yuri%"), "params: {params:?}");
    }

    #[test]
    fn test_compile_limit_in_sql() {
        let q = DslQuery {
            from_type: "evento".to_string(),
            from_types: vec![],
            filters: vec![],
            limit: 42,
        };
        let (sql, _) = compile(&q, "u1");
        assert!(sql.contains("LIMIT 42"), "sql: {sql}");
    }

    #[test]
    fn test_compile_multiple_includes_uses_distinct_and_multiple_joins() {
        let q =
            parse(r#"FROM evento WHERE attendees INCLUDES "yuri" AND attendees INCLUDES "ana""#)
                .unwrap();
        let (sql, _) = compile(&q, "u1");
        let join_count = sql.matches("JOIN entry_relations").count();
        assert_eq!(join_count, 2, "two INCLUDES = two JOINs, sql: {sql}");
        assert!(sql.contains("DISTINCT"));
    }

    #[test]
    fn test_compile_category_uses_in_clause() {
        let cats = default_type_categories();
        let q = resolve(parse("type:music").unwrap(), &cats);
        let (sql, params) = compile(&q, "u1");
        assert!(
            sql.contains("entry_type IN"),
            "category should use IN clause, sql: {sql}"
        );
        assert!(params.contains(&"song".to_string()));
        assert!(params.contains(&"album".to_string()));
    }

    #[test]
    fn test_compile_wildcard_type_omits_type_filter() {
        let q = parse(r#"caderno_id:"black-2024""#).unwrap();
        let (sql, _) = compile(&q, "u1");
        // The SELECT list always includes e.entry_type; we only check that
        // neither `entry_type =` nor `entry_type IN` appears in the WHERE.
        assert!(
            !sql.contains("entry_type =") && !sql.contains("entry_type IN"),
            "wildcard from_type should omit type filter, sql: {sql}"
        );
    }

    #[test]
    fn test_compile_is_not_null() {
        let q = parse("FROM song WHERE references.youtube IS NOT NULL").unwrap();
        let (sql, _) = compile(&q, "u1");
        assert!(
            sql.contains("IS NOT NULL"),
            "IS NOT NULL must appear in SQL, sql: {sql}"
        );
        assert!(
            sql.contains("$.references.youtube"),
            "dotted path must be in json_extract, sql: {sql}"
        );
    }

    #[test]
    fn test_compile_date_before_uses_column() {
        let q = parse("type:notas AND before:2026-05-01").unwrap();
        let (sql, params) = compile(&q, "u1");
        assert!(
            sql.contains("e.created_at < "),
            "before: should filter on created_at column, sql: {sql}"
        );
        assert!(params.contains(&"2026-05-01".to_string()));
    }

    #[test]
    fn test_compile_date_after_uses_column() {
        let q = parse("type:notas AND after:2025-01-01").unwrap();
        let (sql, params) = compile(&q, "u1");
        assert!(
            sql.contains("e.created_at > "),
            "after: should filter on created_at column, sql: {sql}"
        );
        assert!(params.contains(&"2025-01-01".to_string()));
    }
}

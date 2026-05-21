//! CO-74: Query DSL — simple SQL-like language compiled to parameterized SQLite.
//!
//! # Syntax
//!
//! ```text
//! FROM <type> [WHERE <cond> [AND <cond>]*] [LIMIT <n>]
//!
//! <cond> ::= <field> = <quoted>         -- exact frontmatter match
//!          | <field> LIKE <quoted>       -- LIKE frontmatter match
//!          | <field> INCLUDES <quoted>   -- entry_relations join (to_path LIKE %val%)
//! ```
//!
//! # Example
//!
//! ```text
//! FROM evento WHERE attendees INCLUDES "yuri" AND status = "confirmed" LIMIT 50
//! ```
//!
//! # Safety
//!
//! All user-supplied values are bound via SQLite parameters (`?N`).  Field names
//! are validated as safe identifiers (alphanumeric + `_` + `-`) before being
//! interpolated into `json_extract` paths — they are never placed directly in
//! the SQL identifier position.
//!
//! # Scale
//!
//! Results are capped at 1 000 rows (CO-74 scale note).  An explicit `LIMIT`
//! is clamped to this value silently.

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
                "unsafe field name '{name}': must contain only alphanumeric chars, '-', or '_'"
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
    pub from_type: String,
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
}

// ---------------------------------------------------------------------------
// Parser
// ---------------------------------------------------------------------------

/// Parse a DSL query string into an [`DslQuery`] AST.
pub fn parse(input: &str) -> Result<DslQuery, QueryError> {
    let tokens = tokenize(input)?;
    parse_tokens(&tokens)
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
        filters,
        limit,
    })
}

fn parse_condition(tokens: &[Token], pos: &mut usize) -> Result<DslFilter, QueryError> {
    let field = take_word(tokens, pos, "field name")?;
    validate_field_name(&field)?;

    let op = take_word(tokens, pos, "operator (=, LIKE, INCLUDES)")?;

    let value = match tokens.get(*pos) {
        Some(Token::Quoted(s)) => {
            let s = s.clone();
            *pos += 1;
            s
        }
        Some(Token::Word(s)) => {
            let s = s.clone();
            *pos += 1;
            s
        }
        Some(Token::Number(n)) => {
            let s = n.to_string();
            *pos += 1;
            s
        }
        None => {
            return Err(QueryError::Parse(format!(
                "expected value after operator '{op}', got end of input"
            )));
        }
    };

    match op.to_ascii_uppercase().as_str() {
        "=" => Ok(DslFilter::FieldEq { field, value }),
        "LIKE" => Ok(DslFilter::FieldLike {
            field,
            pattern: value,
        }),
        "INCLUDES" => Ok(DslFilter::RelationIncludes {
            field,
            target: value,
        }),
        other => Err(QueryError::Parse(format!(
            "unknown operator '{other}', expected =, LIKE, or INCLUDES"
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

/// Field names coming from the DSL are interpolated into `json_extract` paths.
/// Only alphanumeric characters, `_`, and `-` are permitted.
fn validate_field_name(field: &str) -> Result<(), QueryError> {
    if field.is_empty()
        || !field
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        Err(QueryError::UnsafeFieldName(field.to_string()))
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// SQL compiler
// ---------------------------------------------------------------------------

/// Compile a [`DslQuery`] to a parameterized SQLite `(sql, params)` pair.
///
/// `?1` is always the `universe_key`.  The returned `params` vec is aligned
/// with the `?N` placeholders in the SQL string.
pub fn compile(query: &DslQuery, universe_key: &str) -> (String, Vec<String>) {
    let mut params: Vec<String> = vec![universe_key.to_string()];
    let mut conditions: Vec<String> = vec!["e.universe_key = ?1".to_string()];

    let type_idx = push_param(&mut params, &query.from_type);
    conditions.push(format!("e.entry_type = ?{type_idx}"));

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
                conditions.push(format!(
                    "json_extract(e.frontmatter_json, '$.{field}') = ?{idx}"
                ));
            }
            DslFilter::FieldLike { field, pattern } => {
                let idx = push_param(&mut params, pattern);
                conditions.push(format!(
                    "json_extract(e.frontmatter_json, '$.{field}') LIKE ?{idx}"
                ));
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

    // ---- parse ----

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

    // ---- compile ----

    #[test]
    fn test_compile_minimal() {
        let q = DslQuery {
            from_type: "evento".to_string(),
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
        // to_path pattern should be %yuri%
        assert!(params.iter().any(|p| p == "%yuri%"), "params: {params:?}");
    }

    #[test]
    fn test_compile_limit_in_sql() {
        let q = DslQuery {
            from_type: "evento".to_string(),
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
        // Should have two JOIN clauses
        let join_count = sql.matches("JOIN entry_relations").count();
        assert_eq!(join_count, 2, "two INCLUDES = two JOINs, sql: {sql}");
        assert!(sql.contains("DISTINCT"));
    }
}

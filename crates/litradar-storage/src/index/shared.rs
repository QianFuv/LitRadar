//! Shared index connection, pagination, sorting, and query helpers.

use super::*;

pub(super) fn open_index_connection(
    config: &StorageConfig,
    db_name: Option<&str>,
) -> Result<Connection, IndexRepositoryError> {
    let db_path = config.resolve_index_db_path(db_name)?;
    let connection = open_sqlite_connection(db_path)?;
    let extension_path = config.simple_tokenizer_path();
    try_load_extension(&connection, extension_path.as_deref())?;
    Ok(connection)
}

pub(super) fn is_article_listing_ready(connection: &Connection) -> bool {
    let status = connection
        .query_row("SELECT status FROM listing_state WHERE id = 1", [], |row| {
            row.get::<_, String>(0)
        })
        .optional();
    if !matches!(status, Ok(Some(value)) if value == "ready") {
        return false;
    }
    connection
        .query_row("SELECT 1 FROM article_listing LIMIT 1", [], |row| {
            row.get::<_, i64>(0)
        })
        .is_ok()
}

pub(super) fn page_meta(
    total: Option<i64>,
    limit: i64,
    offset: i64,
    next_cursor: Option<String>,
    has_more: Option<bool>,
) -> PageMeta {
    PageMeta {
        total,
        limit,
        offset,
        next_cursor,
        has_more,
    }
}

pub(super) fn sort_sql(
    sort: &str,
    allowed: &[(&str, &str)],
) -> Result<String, IndexRepositoryError> {
    let specs = sort_specs(sort, allowed)?;
    if specs.is_empty() {
        return Ok(String::new());
    }
    Ok(format!(
        "ORDER BY {}",
        specs
            .into_iter()
            .map(|spec| format!("{} {}", spec.column, spec.direction.sql()))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

pub(super) fn sort_specs(
    sort: &str,
    allowed: &[(&str, &str)],
) -> Result<Vec<SortSpec>, IndexRepositoryError> {
    let mut specs = Vec::new();
    for raw_part in sort.split(',') {
        let part = raw_part.trim();
        if part.is_empty() {
            continue;
        }
        let (field, direction) = if let Some(field) = part.strip_prefix('-') {
            (field.trim(), SortDirection::Desc)
        } else if let Some((field, raw_direction)) = part.split_once(':') {
            let direction = if raw_direction.trim().eq_ignore_ascii_case("desc") {
                SortDirection::Desc
            } else {
                SortDirection::Asc
            };
            (field.trim(), direction)
        } else {
            (part, SortDirection::Asc)
        };
        let Some((_, column)) = allowed.iter().find(|(name, _)| *name == field) else {
            return Err(IndexRepositoryError::UnsupportedSortField(
                field.to_string(),
            ));
        };
        specs.push(SortSpec {
            column: column.to_string(),
            direction,
        });
    }
    Ok(specs)
}

pub(super) fn push_cursor_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    alias: &str,
    direction: SortDirection,
    cursor: Option<&str>,
) -> Result<(), IndexRepositoryError> {
    let Some(cursor) = cursor else {
        return Ok(());
    };
    let (date, article_id) = parse_article_cursor(cursor)?;
    let operator = if direction == SortDirection::Desc {
        "<"
    } else {
        ">"
    };
    clauses.push(format!(
        "({alias}.date {operator} ? OR ({alias}.date = ? AND {alias}.article_id {operator} ?))"
    ));
    values.push(SqlValue::Text(date.clone()));
    values.push(SqlValue::Text(date));
    values.push(SqlValue::Integer(article_id));
    Ok(())
}

pub(super) fn parse_article_cursor(cursor: &str) -> Result<(String, i64), IndexRepositoryError> {
    let Some((date, article_id)) = cursor.split_once('|') else {
        return Err(IndexRepositoryError::InvalidCursor);
    };
    if date.is_empty() {
        return Err(IndexRepositoryError::InvalidCursor);
    }
    let article_id = article_id
        .parse::<i64>()
        .map_err(|_| IndexRepositoryError::InvalidCursor)?;
    Ok((date.to_string(), article_id))
}

pub(super) fn push_fts_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    column: &str,
    q: &Option<String>,
) {
    if let Some(query) = nonempty(q.as_deref()) {
        clauses.push(format!(
            "{column} IN (SELECT rowid FROM article_search WHERE article_search MATCH ?)"
        ));
        values.push(SqlValue::Text(query.to_string()));
    }
}

pub(super) fn push_int_list_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    column: &str,
    items: &[i64],
) {
    if items.is_empty() {
        return;
    }
    clauses.push(format!("{column} IN ({})", placeholders(items.len())));
    values.extend(items.iter().copied().map(SqlValue::Integer));
}

pub(super) fn push_string_list_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    column: &str,
    items: &[String],
) {
    if items.is_empty() {
        return;
    }
    clauses.push(format!("{column} IN ({})", placeholders(items.len())));
    values.extend(items.iter().cloned().map(SqlValue::Text));
}

pub(super) fn push_optional_int_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    clause: &str,
    value: Option<i64>,
) {
    if let Some(value) = value {
        clauses.push(clause.to_string());
        values.push(SqlValue::Integer(value));
    }
}

pub(super) fn push_optional_bool_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    clause: &str,
    value: Option<bool>,
) {
    if let Some(value) = value {
        clauses.push(clause.to_string());
        values.push(SqlValue::Integer(value as i64));
    }
}

pub(super) fn push_optional_text_filter(
    clauses: &mut Vec<String>,
    values: &mut Vec<SqlValue>,
    clause: &str,
    value: &Option<String>,
) {
    if let Some(value) = nonempty(value.as_deref()) {
        clauses.push(clause.to_string());
        values.push(SqlValue::Text(value.to_string()));
    }
}

pub(super) fn where_sql(clauses: &[String]) -> String {
    if clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", clauses.join(" AND "))
    }
}

pub(super) fn placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(", ")
}

pub(super) fn validate_limit_offset(limit: i64, offset: i64) -> Result<(), IndexRepositoryError> {
    if !(1..=MAX_LIMIT).contains(&limit) {
        return Err(IndexRepositoryError::InvalidPagination(
            "limit must be between 1 and 200",
        ));
    }
    if offset < 0 {
        return Err(IndexRepositoryError::InvalidPagination(
            "offset must be greater than or equal to 0",
        ));
    }
    Ok(())
}

pub(super) fn nonempty(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

pub(super) fn nonempty_owned(value: Option<String>) -> Option<String> {
    value
        .map(|item| item.trim().to_string())
        .filter(|item| !item.is_empty())
}

pub(super) fn collect_rows<T>(
    rows: rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>>,
) -> Result<Vec<T>, IndexRepositoryError> {
    let mut items = Vec::new();
    for row in rows {
        items.push(row?);
    }
    Ok(items)
}

#[derive(Debug, Clone)]
pub(super) struct SortSpec {
    pub(super) column: String,
    pub(super) direction: SortDirection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SortDirection {
    Asc,
    Desc,
}

impl SortDirection {
    pub(super) fn sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

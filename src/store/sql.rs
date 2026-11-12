use crate::error::AppResult;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dialect {
    Sqlite,
    Postgres,
}

impl Dialect {
    pub(crate) fn render(self, sql: &str) -> String {
        match self {
            Dialect::Sqlite => sql.to_string(),
            Dialect::Postgres => rewrite(sql),
        }
    }
}

pub(crate) fn rewrite(sql: &str) -> String {
    let bytes = sql.as_bytes();
    let mut out = String::with_capacity(sql.len());
    let mut i = 0;
    while i < bytes.len() {
        let Some(ch) = sql[i..].chars().next() else {
            break;
        };
        if ch == '\'' || ch == '"' {
            let end = quoted_end(bytes, i);
            out.push_str(&sql[i..end]);
            i = end;
            continue;
        }
        if sql[i..].starts_with("--") {
            let end = sql[i..]
                .find('\n')
                .map_or(sql.len(), |offset| i + offset + 1);
            out.push_str(&sql[i..end]);
            i = end;
            continue;
        }
        if sql[i..].starts_with("/*") {
            let end = block_comment_end(bytes, i);
            out.push_str(&sql[i..end]);
            i = end;
            continue;
        }
        if ch == '$' {
            if let Some(tag) = dollar_tag(sql, i) {
                let content = i + tag.len();
                if let Some(offset) = sql[content..].find(tag) {
                    let end = content + offset + tag.len();
                    out.push_str(&sql[i..end]);
                    i = end;
                    continue;
                }
            }
        }
        if ch == '?' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit) {
            out.push('$');
            i += 1;
            while let Some(next) = sql[i..].chars().next() {
                if !next.is_ascii_digit() {
                    break;
                }
                out.push(next);
                i += next.len_utf8();
            }
            continue;
        }
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

fn quoted_end(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == quote {
            if bytes.get(i + 1) == Some(&quote) {
                i += 2;
            } else {
                return i + 1;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

fn block_comment_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0;
    let mut i = start;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"/*") {
            depth += 1;
            i += 2;
        } else if bytes[i..].starts_with(b"*/") {
            depth -= 1;
            i += 2;
            if depth == 0 {
                return i;
            }
        } else {
            i += 1;
        }
    }
    bytes.len()
}

fn dollar_tag(sql: &str, start: usize) -> Option<&str> {
    let rest = &sql[start..];
    let mut end = None;
    for (offset, ch) in rest.char_indices().skip(1) {
        if ch == '$' {
            end = Some(offset + 1);
            break;
        }
        let first = offset == 1;
        if !(ch.is_ascii_alphabetic() || ch == '_' || (!first && ch.is_ascii_digit())) {
            return None;
        }
    }
    let end = end?;
    Some(&rest[..end])
}

pub(crate) fn postgres(sql: &str) -> String {
    Dialect::Postgres.render(sql)
}

pub(crate) trait SqlRow {
    fn get<T: Column>(&self, index: usize) -> AppResult<T>;
}

impl SqlRow for rusqlite::Row<'_> {
    fn get<T: Column>(&self, index: usize) -> AppResult<T> {
        T::from_sqlite(self, index)
    }
}

impl SqlRow for tokio_postgres::Row {
    fn get<T: Column>(&self, index: usize) -> AppResult<T> {
        T::from_postgres(self, index)
    }
}

pub(crate) trait Column: Sized {
    fn from_sqlite(row: &rusqlite::Row<'_>, index: usize) -> AppResult<Self>;
    fn from_postgres(row: &tokio_postgres::Row, index: usize) -> AppResult<Self>;
}

impl Column for String {
    fn from_sqlite(row: &rusqlite::Row<'_>, index: usize) -> AppResult<Self> {
        Ok(row.get(index)?)
    }

    fn from_postgres(row: &tokio_postgres::Row, index: usize) -> AppResult<Self> {
        Ok(row.try_get(index)?)
    }
}

impl Column for Option<String> {
    fn from_sqlite(row: &rusqlite::Row<'_>, index: usize) -> AppResult<Self> {
        Ok(row.get(index)?)
    }

    fn from_postgres(row: &tokio_postgres::Row, index: usize) -> AppResult<Self> {
        Ok(row.try_get(index)?)
    }
}

impl Column for i64 {
    fn from_sqlite(row: &rusqlite::Row<'_>, index: usize) -> AppResult<Self> {
        Ok(row.get(index)?)
    }

    fn from_postgres(row: &tokio_postgres::Row, index: usize) -> AppResult<Self> {
        Ok(row.try_get(index)?)
    }
}

impl Column for i32 {
    fn from_sqlite(row: &rusqlite::Row<'_>, index: usize) -> AppResult<Self> {
        Ok(row.get(index)?)
    }

    fn from_postgres(row: &tokio_postgres::Row, index: usize) -> AppResult<Self> {
        Ok(row.try_get(index)?)
    }
}

impl Column for bool {
    fn from_sqlite(row: &rusqlite::Row<'_>, index: usize) -> AppResult<Self> {
        let value: i64 = row.get(index)?;
        Ok(value != 0)
    }

    fn from_postgres(row: &tokio_postgres::Row, index: usize) -> AppResult<Self> {
        let value: i32 = row.try_get(index)?;
        Ok(value != 0)
    }
}

pub(crate) fn sqlite_opt<T, F>(
    conn: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
    map: F,
) -> AppResult<Option<T>>
where
    F: for<'a, 'b> Fn(&'a rusqlite::Row<'b>) -> AppResult<T>,
{
    let mut statement = conn.prepare(sql)?;
    let mut rows = statement.query(params)?;
    let Some(row) = rows.next()? else {
        return Ok(None);
    };
    Ok(Some(map(row)?))
}

pub(crate) fn sqlite_all<T, F>(
    conn: &rusqlite::Connection,
    sql: &str,
    params: impl rusqlite::Params,
    map: F,
) -> AppResult<Vec<T>>
where
    F: for<'a, 'b> Fn(&'a rusqlite::Row<'b>) -> AppResult<T>,
{
    let mut statement = conn.prepare(sql)?;
    let mut rows = statement.query(params)?;
    let mut items = Vec::new();
    while let Some(row) = rows.next()? {
        items.push(map(row)?);
    }
    Ok(items)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewrites_numbered_placeholders() {
        assert_eq!(
            rewrite("SELECT * FROM users WHERE id = ?1 AND name = ?2"),
            "SELECT * FROM users WHERE id = $1 AND name = $2"
        );
        assert_eq!(rewrite("VALUES (?10, ?2)"), "VALUES ($10, $2)");
    }

    #[test]
    fn leaves_question_marks_in_string_literals_alone() {
        assert_eq!(
            rewrite("SELECT '?' AS mark, ?1, 'it''s ?2' AS text"),
            "SELECT '?' AS mark, $1, 'it''s ?2' AS text"
        );
        assert_eq!(
            rewrite("SELECT '?10' AS literal, ?1"),
            "SELECT '?10' AS literal, $1"
        );
    }

    #[test]
    fn leaves_question_marks_in_other_quoted_regions_alone() {
        assert_eq!(
            rewrite("SELECT \"?1\" AS name, ?2"),
            "SELECT \"?1\" AS name, $2"
        );
        assert_eq!(
            rewrite("SELECT ?1 -- ?2\nFROM users"),
            "SELECT $1 -- ?2\nFROM users"
        );
        assert_eq!(
            rewrite("SELECT ?1 /* ?2 /* ?3 */ ?4 */ FROM users"),
            "SELECT $1 /* ?2 /* ?3 */ ?4 */ FROM users"
        );
    }

    #[test]
    fn leaves_question_marks_in_dollar_quoted_bodies_alone() {
        assert_eq!(rewrite("SELECT $$?1$$, ?2"), "SELECT $$?1$$, $2");
        assert_eq!(
            rewrite("SELECT $tag$?1 $2$tag$, ?3"),
            "SELECT $tag$?1 $2$tag$, $3"
        );
    }

    #[test]
    fn keeps_non_ascii_text_intact() {
        assert_eq!(
            rewrite("SELECT 'café ?1' AS text, ?2"),
            "SELECT 'café ?1' AS text, $2"
        );
    }

    #[test]
    fn leaves_unquoted_question_marks_without_digits_alone() {
        assert_eq!(rewrite("SELECT ?"), "SELECT ?");
        assert_eq!(rewrite("SELECT key ? 'value'"), "SELECT key ? 'value'");
    }

    #[test]
    fn sqlite_render_keeps_question_marks() {
        assert_eq!(Dialect::Sqlite.render("SELECT ?1"), "SELECT ?1".to_string());
        assert_eq!(
            Dialect::Postgres.render("SELECT ?1"),
            "SELECT $1".to_string()
        );
    }
}

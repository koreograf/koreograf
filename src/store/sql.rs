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
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut quoted = false;
    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                out.push('\'');
                if quoted && chars.peek() == Some(&'\'') {
                    out.push('\'');
                    chars.next();
                } else {
                    quoted = !quoted;
                }
            }
            '?' if !quoted && chars.peek().is_some_and(char::is_ascii_digit) => {
                out.push('$');
                while let Some(next) = chars.next_if(char::is_ascii_digit) {
                    out.push(next);
                }
            }
            _ => out.push(ch),
        }
    }
    out
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

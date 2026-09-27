/// Result metadata for a Turso query execution
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct TursoQueryResult {
    rows_affected: u64,
    last_insert_rowid: i64,
}

impl TursoQueryResult {
    /// Creates query result metadata
    #[must_use]
    pub fn new(rows_affected: u64, last_insert_rowid: i64) -> Self {
        Self {
            rows_affected,
            last_insert_rowid,
        }
    }

    /// Returns the number of rows affected by the query
    #[must_use]
    pub fn rows_affected(&self) -> u64 {
        self.rows_affected
    }

    /// Returns the rowid produced by the most recent insert on the connection
    #[must_use]
    pub fn last_insert_rowid(&self) -> i64 {
        self.last_insert_rowid
    }
}

impl Extend<TursoQueryResult> for TursoQueryResult {
    fn extend<T>(&mut self, iter: T)
    where
        T: IntoIterator<Item = TursoQueryResult>,
    {
        for result in iter {
            self.rows_affected += result.rows_affected;
            self.last_insert_rowid = result.last_insert_rowid;
        }
    }
}

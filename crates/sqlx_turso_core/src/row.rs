use std::sync::Arc;

use sqlx_core::{
    HashMap, column::ColumnIndex, error::Error, ext::ustr::UStr, impl_column_index_for_row,
    row::Row,
};

use crate::{
    Turso, TursoColumn, TursoValue, TursoValueRef, column::collect_column_names,
    prepared_rows::RowValues,
};

#[derive(Clone)]
enum Values {
    Native(Vec<TursoValue>),
    Prepared(Arc<RowValues>),
}
impl std::fmt::Debug for Values {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(&**self, f)
    }
}
impl Default for Values {
    fn default() -> Self {
        Self::Native(Vec::new())
    }
}
impl std::ops::Deref for Values {
    type Target = [TursoValue];
    fn deref(&self) -> &[TursoValue] {
        match self {
            Self::Native(v) => v,
            Self::Prepared(v) => &v.values,
        }
    }
}

/// Row returned from a Turso query
#[derive(Clone, Debug, Default)]
pub struct TursoRow {
    columns: Arc<[TursoColumn]>,
    column_names: Arc<HashMap<UStr, usize>>,
    values: Values,
}

impl TursoRow {
    /// Creates a row from column metadata and values
    #[must_use]
    pub fn new(columns: Vec<TursoColumn>, values: Vec<TursoValue>) -> Self {
        let columns: Arc<[TursoColumn]> = columns.into();
        let column_names = collect_column_names(&columns);

        Self {
            columns,
            column_names,
            values: Values::Native(values),
        }
    }

    pub(crate) fn with_shared_columns(
        columns: Arc<[TursoColumn]>,
        column_names: Arc<HashMap<UStr, usize>>,
        values: Vec<TursoValue>,
    ) -> Self {
        Self {
            columns,
            column_names,
            values: Values::Native(values),
        }
    }

    pub(crate) fn prepared(
        columns: Arc<[TursoColumn]>,
        column_names: Arc<HashMap<UStr, usize>>,
        values: Arc<RowValues>,
    ) -> Self {
        Self {
            columns,
            column_names,
            values: Values::Prepared(values),
        }
    }

    #[cfg(feature = "any")]
    pub(crate) fn column_names(&self) -> Arc<HashMap<UStr, usize>> {
        self.column_names.clone()
    }
}

impl Row for TursoRow {
    type Database = Turso;

    fn columns(&self) -> &[TursoColumn] {
        &self.columns
    }

    fn try_get_raw<I>(&self, index: I) -> Result<TursoValueRef<'_>, Error>
    where
        I: ColumnIndex<Self>,
    {
        let index = index.index(self)?;
        self.values
            .get(index)
            .map(TursoValueRef::new)
            .ok_or(Error::ColumnIndexOutOfBounds {
                len: self.values.len(),
                index,
            })
    }
}

impl_column_index_for_row!(TursoRow);

impl ColumnIndex<TursoRow> for str {
    fn index(&self, row: &TursoRow) -> Result<usize, Error> {
        row.column_names
            .get(self)
            .copied()
            .ok_or_else(|| Error::ColumnNotFound(self.to_owned()))
    }
}

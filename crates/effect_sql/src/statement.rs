//! Inert SQL descriptions separate caller-reviewed syntax from bound values.

use crate::SqlError;
use sqlx::{Arguments, Encode, Turso, TursoArguments, Type, Value};
use std::collections::HashSet;

/// Validated identifier path; each component is quoted with SQLite/Turso double quotes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[must_use]
pub struct Identifier {
    /// Quoted immutable syntax, never an application value.
    quoted: String,
}
impl Identifier {
    /// Validate and quote a schema/table/column path. Quotes are escaped, NUL/empty parts rejected.
    pub fn new(value: impl AsRef<str>) -> Result<Self, SqlError> {
        let value = value.as_ref();
        if value.contains('\0') || value.split('.').any(|part| part.trim().is_empty()) {
            return Err(SqlError::Identifier(value.to_owned()));
        }
        Ok(Self {
            quoted: value
                .split('.')
                .map(|part| format!("\"{}\"", part.replace('"', "\"\"")))
                .collect::<Vec<_>>()
                .join("."),
        })
    }
    /// Borrow the quoted SQL representation for trusted syntax assembly.
    #[must_use]
    pub fn quoted(&self) -> &str {
        &self.quoted
    }
}

/// Explicit caller-reviewed SQL syntax. Dynamic text is trusted by the caller, never escaped as data.
#[derive(Clone, Debug)]
#[must_use]
pub struct TrustedSql(pub(crate) String);
impl TrustedSql {
    /// Validate text for NUL; SQL correctness is still the native provider's responsibility.
    pub fn new(text: impl Into<String>) -> Result<Self, SqlError> {
        let text = text.into();
        if text.contains('\0') {
            return Err(SqlError::Configuration("SQL text contains NUL".into()));
        }
        Ok(Self(text))
    }
}

/// Owned compiled statement. Values move once into SQLx; no Clone bound or payload copy is imposed.
#[must_use]
pub struct SqlStatement {
    /// Provider SQL, kept out of public Debug and telemetry.
    pub(crate) text: String,
    /// Ordered typed native bindings, owned by this execution.
    pub(crate) arguments: TursoArguments,
}
impl SqlStatement {
    /// Start an inert statement builder. Descriptions are assembled within an execution for moved payloads.
    pub fn builder() -> StatementBuilder {
        StatementBuilder {
            text: String::new(),
            arguments: TursoArguments::default(),
            failure: None,
        }
    }
    /// Construct a parameter-free caller-reviewed native statement.
    pub fn trusted(text: impl Into<String>) -> Result<Self, SqlError> {
        Self::builder().text(text).finish()
    }
    /// Borrow native SQL only for explicit checked/native interoperation; never log it by default.
    #[must_use]
    pub fn sql(&self) -> &str {
        &self.text
    }
    /// Consume the description for direct SQLx use with the same ordered native bindings.
    #[must_use]
    pub fn into_parts(self) -> (String, TursoArguments) {
        (self.text, self.arguments)
    }
}

/// Ordered builder carrying its first validation error without panicking in macro expansion.
#[must_use]
pub struct StatementBuilder {
    /// SQL assembled in declaration order.
    text: String,
    /// Bindings assembled in declaration order.
    arguments: TursoArguments,
    /// First validation failure; subsequent operations retain its precedence.
    failure: Option<SqlError>,
}
impl StatementBuilder {
    /// Append explicitly trusted syntax; no I/O occurs.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        match TrustedSql::new(text) {
            Ok(text) => self.text.push_str(&text.0),
            Err(error) => self.record(error),
        }
        self
    }
    /// Append a quoted identifier; values must use param instead.
    pub fn identifier(mut self, identifier: impl AsRef<str>) -> Self {
        match Identifier::new(identifier) {
            Ok(identifier) => self.text.push_str(identifier.quoted()),
            Err(error) => self.record(error),
        }
        self
    }
    /// Encode one native binding and append an anonymous placeholder preserving insertion order.
    pub fn param<T>(mut self, value: T) -> Self
    where
        T: Type<Turso>,
        for<'q> T: Encode<'q, Turso>,
    {
        if self.failure.is_none() {
            match self.arguments.add(value) {
                Ok(()) => self.text.push('?'),
                Err(error) => self.record(SqlError::Binding(error)),
            }
        }
        self
    }
    /// Append an already owned fragment, moving its bindings without cloning cells.
    pub fn fragment(mut self, fragment: SqlStatement) -> Self {
        self.text.push_str(&fragment.text);
        self.arguments.append(fragment.arguments);
        self
    }
    /// Append one typed WHERE predicate; guarded mutation uses its own exactly-one grammar.
    pub fn predicate(self, predicate: SqlPredicate) -> Self {
        self.text(" WHERE ").fragment(predicate.0)
    }
    /// Return the checked owned statement, preserving the first configuration error.
    pub fn finish(self) -> Result<SqlStatement, SqlError> {
        if let Some(error) = self.failure {
            return Err(error);
        }
        if self.text.trim().is_empty() {
            return Err(SqlError::Configuration("empty SQL statement".into()));
        }
        Ok(SqlStatement {
            text: self.text,
            arguments: self.arguments,
        })
    }
    /// Retain deterministic validation precedence without replacing an earlier diagnostic.
    fn record(&mut self, error: SqlError) {
        if self.failure.is_none() {
            self.failure = Some(error);
        }
    }
}

/// Typed predicate; guarded updates cannot accept arbitrary missing WHERE text.
#[must_use]
pub struct SqlPredicate(pub(crate) SqlStatement);
impl SqlPredicate {
    /// Bind equality, emitting IS NULL for an explicitly encoded SQL NULL.
    pub fn equal<T>(column: impl AsRef<str>, value: T) -> Result<Self, SqlError>
    where
        T: Type<Turso>,
        for<'q> T: Encode<'q, Turso>,
    {
        let column = Identifier::new(column)?;
        let mut arguments = TursoArguments::default();
        arguments.add(value).map_err(SqlError::Binding)?;
        let null = arguments.values().first().is_some_and(Value::is_null);
        Ok(Self(SqlStatement {
            text: format!(
                "{} {}",
                column.quoted(),
                if null { "IS NULL" } else { "= ?" }
            ),
            arguments: if null {
                TursoArguments::default()
            } else {
                arguments
            },
        }))
    }
    /// Use an explicit trusted predicate fragment whose placeholders already carry bindings.
    pub fn trusted(statement: SqlStatement) -> Self {
        Self(statement)
    }
    /// Combine predicates while preserving native parameter ordering.
    pub fn and(self, other: Self) -> Result<Self, SqlError> {
        Ok(Self(
            SqlStatement::builder()
                .text("(")
                .fragment(self.0)
                .text(") AND (")
                .fragment(other.0)
                .text(")")
                .finish()?,
        ))
    }
}

/// Checked insert/update collector used by macros; duplicate columns fail before I/O.
#[must_use]
pub struct MutationBuilder {
    /// Validated table or first failure.
    table: Result<Identifier, SqlError>,
    /// Unique quoted assignments preserving declaration order.
    columns: Vec<Identifier>,
    /// Native binding payloads moved in declaration order.
    arguments: TursoArguments,
    /// Duplicate and encoding failures retained without macro unwraps.
    failure: Option<SqlError>,
    /// Native quoted identities already seen.
    seen: HashSet<String>,
}
impl MutationBuilder {
    /// Start a nonexecuting mutation description for one table.
    pub fn new(table: impl AsRef<str>) -> Self {
        Self {
            table: Identifier::new(table),
            columns: Vec::new(),
            arguments: TursoArguments::default(),
            failure: None,
            seen: HashSet::new(),
        }
    }
    /// Move one assignment into the description; native encoders retain original value types.
    pub fn value<T>(mut self, column: impl AsRef<str>, value: T) -> Self
    where
        T: Type<Turso>,
        for<'q> T: Encode<'q, Turso>,
    {
        if self.table.is_err() || self.failure.is_some() {
            return self;
        }
        let result = (|| {
            let column = Identifier::new(column)?;
            if !self.seen.insert(column.quoted.clone()) {
                return Err(SqlError::Configuration("duplicate mutation column".into()));
            }
            self.arguments.add(value).map_err(SqlError::Binding)?;
            self.columns.push(column);
            Ok(())
        })();
        if self.failure.is_none() {
            self.failure = result.err();
        }
        self
    }
    /// Finish an INSERT; at least one unique column/value is required.
    pub fn insert(self) -> Result<SqlStatement, SqlError> {
        let table = self.table?;
        if let Some(error) = self.failure {
            return Err(error);
        }
        if self.columns.is_empty() {
            return Err(SqlError::Configuration("insert requires values".into()));
        }
        let columns = self
            .columns
            .iter()
            .map(Identifier::quoted)
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = vec!["?"; self.columns.len()].join(", ");
        Ok(SqlStatement {
            text: format!(
                "INSERT INTO {} ({columns}) VALUES ({placeholders})",
                table.quoted()
            ),
            arguments: self.arguments,
        })
    }
    /// Finish a guarded UPDATE; this API requires one predicate rather than implicit full-table writes.
    pub fn update(mut self, predicate: SqlPredicate) -> Result<SqlStatement, SqlError> {
        let table = self.table?;
        if let Some(error) = self.failure {
            return Err(error);
        }
        if self.columns.is_empty() {
            return Err(SqlError::Configuration(
                "update requires assignments".into(),
            ));
        }
        let assignments = self
            .columns
            .iter()
            .map(|column| format!("{} = ?", column.quoted()))
            .collect::<Vec<_>>()
            .join(", ");
        self.arguments.append(predicate.0.arguments);
        Ok(SqlStatement {
            text: format!(
                "UPDATE {} SET {assignments} WHERE {}",
                table.quoted(),
                predicate.0.text
            ),
            arguments: self.arguments,
        })
    }
}

//! Inert SQL descriptions separate caller-reviewed syntax from bound values.

use crate::SqlError;
use crate::sql;
use effect::memory::{AdmissionBundle, MemoryError};
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
    #[expect(clippy::result_large_err, reason = "retains the owned SQL error representation without an extra heap allocation")]
    pub fn new(value: impl AsRef<str>) -> Result<Self, SqlError> {
        let value = value.as_ref();
        if value.contains('\0') || value.split('.').any(|part| part.trim().is_empty()) {
            let mut invalid = String::new();
            append_sql(&mut invalid, &[value])?;
            return Err(SqlError::Identifier(invalid));
        }
        let mut quote_bytes = 2usize;
        for byte in value.bytes() {
            let extra = match byte { b'.' => 2, b'"' => 1, _ => 0 };
            quote_bytes = quote_bytes.checked_add(extra).ok_or(SqlError::Memory(MemoryError::ArithmeticOverflow))?;
        }
        let size = value.len().checked_add(quote_bytes).ok_or(SqlError::Memory(MemoryError::ArithmeticOverflow))?;
        let mut quoted = String::new();
        quoted.try_reserve_exact(size).map_err(|_| SqlError::Memory(MemoryError::CollectionReservationFailed))?;
        for (index, part) in value.split('.').enumerate() {
            if index != 0 { quoted.push('.'); }
            quoted.push('"');
            for character in part.chars() {
                if character == '"' { quoted.push('"'); }
                quoted.push(character);
            }
            quoted.push('"');
        }
        Ok(Self { quoted })
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
    #[expect(clippy::result_large_err, reason = "retains the owned SQL error representation without an extra heap allocation")]
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
    pub(crate) retention: StatementRetention,
}

pub(crate) type MaterializationCharge = AdmissionBundle<'static, 1>;

#[derive(Default)]
pub(crate) struct StatementRetention {
    pub(crate) prepared_source: Option<crate::PreparedSqlStatement>,
    charges: MaterializationCharges,
}

#[derive(Default)]
struct MaterializationCharges {
    first: Option<MaterializationCharge>,
    rest: Vec<MaterializationCharge>,
}

impl MaterializationCharges {
    fn append(&mut self, other: &mut Self) -> Result<(), MemoryError> {
        if self.first.is_none() {
            *self = std::mem::take(other);
            return Ok(());
        }
        let additional = other.rest.len().checked_add(usize::from(other.first.is_some()))
            .ok_or(MemoryError::ArithmeticOverflow)?;
        self.rest.try_reserve(additional).map_err(|_| MemoryError::CollectionReservationFailed)?;
        if let Some(first) = other.first.take() {
            self.rest.push(first);
        }
        self.rest.append(&mut other.rest);
        Ok(())
    }
}

impl SqlStatement {
    pub(crate) fn materialized(
        text: String,
        arguments: TursoArguments,
        charge: Option<MaterializationCharge>,
    ) -> Self {
        Self {
            text,
            arguments,
            retention: StatementRetention {
                prepared_source: None,
                charges: MaterializationCharges { first: charge, rest: Vec::new() },
            },
        }
    }

    /// Start an inert statement builder. Descriptions are assembled within an execution for moved payloads.
    pub fn builder() -> StatementBuilder {
        StatementBuilder {
            text: String::new(),
            arguments: TursoArguments::default(),
            failure: None,
            charges: MaterializationCharges::default(),
        }
    }
    /// Construct a parameter-free caller-reviewed native statement.
    #[expect(clippy::result_large_err, reason = "retains the owned SQL error representation without an extra heap allocation")]
    pub fn trusted(text: impl Into<String>) -> Result<Self, SqlError> {
        Self::builder().text(text).finish()
    }
    /// Borrow native SQL only for explicit checked/native interoperation; never log it by default.
    #[must_use]
    pub fn sql(&self) -> &str {
        &self.text
    }
    /// Explicitly transfer copied parts for direct SQLx use. Effect-sql's
    /// prepared-source lease and materialization charges are released here.
    #[must_use]
    pub fn into_parts(self) -> (String, TursoArguments) {
        (self.text, self.arguments)
    }
    pub(crate) fn into_retained_parts(self) -> (String, TursoArguments, StatementRetention) {
        (self.text, self.arguments, self.retention)
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
    /// Charges retained with materialized fragments transferred here.
    charges: MaterializationCharges,
}
impl StatementBuilder {
    /// Append explicitly trusted syntax; no I/O occurs.
    pub fn text(mut self, text: impl Into<String>) -> Self {
        let result = match TrustedSql::new(text) {
            Ok(_) if self.failure.is_some() => Ok(()),
            Ok(text) if self.text.is_empty() => {
                self.text = text.0;
                Ok(())
            }
            Ok(text) => append_sql(&mut self.text, &[&text.0]),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            self.record(error);
        }
        self
    }
    /// Append a quoted identifier; values must use param instead.
    pub fn identifier(mut self, identifier: impl AsRef<str>) -> Self {
        let result = match Identifier::new(identifier) {
            Ok(_) if self.failure.is_some() => Ok(()),
            Ok(identifier) => append_sql(&mut self.text, &[identifier.quoted()]),
            Err(error) => Err(error),
        };
        if let Err(error) = result {
            self.record(error);
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
            let result = match self.arguments.add(value) {
                Ok(()) => append_sql(&mut self.text, &["?"]),
                Err(error) => Err(SqlError::Binding(error)),
            };
            if let Err(error) = result {
                self.record(error);
            }
        }
        self
    }
    /// Append an already owned fragment, moving its bindings without cloning cells.
    #[expect(clippy::result_large_err, reason = "retains the first owned SQL error without boxing")]
    pub fn fragment(mut self, mut fragment: SqlStatement) -> Self {
        if self.failure.is_none() {
            let result = (|| {
                let empty_text = self.text.is_empty() && self.text.capacity() == 0;
                if !empty_text {
                    self.text.try_reserve(fragment.text.len())
                        .map_err(|_| SqlError::Memory(MemoryError::CollectionReservationFailed))?;
                }

                let empty_arguments = self.arguments.len() == 0;
                if !empty_arguments {
                    self.arguments.try_reserve_exact(fragment.arguments.len())
                        .map_err(|_| SqlError::Memory(MemoryError::CollectionReservationFailed))?;
                }
                self.charges.append(&mut fragment.retention.charges)
                    .map_err(SqlError::Memory)?;
                if empty_text {
                    self.text = std::mem::take(&mut fragment.text);
                } else {
                    self.text.push_str(&fragment.text);
                }
                if empty_arguments {
                    self.arguments = std::mem::take(&mut fragment.arguments);
                } else {
                    self.arguments.append(std::mem::take(&mut fragment.arguments));
                }
                Ok(())
            })();
            if let Err(error) = result {
                self.record(error);
            }
        }
        self
    }
    /// Append one typed WHERE predicate; guarded mutation uses its own exactly-one grammar.
    pub fn predicate(self, predicate: SqlPredicate) -> Self {
        self.text(" WHERE ").fragment(predicate.0)
    }
    /// Return the checked owned statement, preserving the first configuration error.
    #[expect(clippy::result_large_err, reason = "retains the owned SQL error representation without an extra heap allocation")]
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
            retention: StatementRetention { prepared_source: None, charges: self.charges },
        })
    }
    /// Retain deterministic validation precedence without replacing an earlier diagnostic.
    fn record(&mut self, error: SqlError) {
        if self.failure.is_none() {
            self.failure = Some(error);
            drop(std::mem::take(&mut self.text));
            drop(std::mem::take(&mut self.arguments));
            drop(std::mem::take(&mut self.charges));
        }
    }
}

/// Typed predicate; guarded updates cannot accept arbitrary missing WHERE text.
#[must_use]
pub struct SqlPredicate(pub(crate) SqlStatement);
impl SqlPredicate {
    /// Bind equality, emitting IS NULL for an explicitly encoded SQL NULL.
    #[expect(clippy::result_large_err, reason = "retains the owned SQL error representation without an extra heap allocation")]
    pub fn equal<T>(column: impl AsRef<str>, value: T) -> Result<Self, SqlError>
    where
        T: Type<Turso>,
        for<'q> T: Encode<'q, Turso>,
    {
        let column = Identifier::new(column)?;
        let mut arguments = TursoArguments::default();
        arguments.add(value).map_err(SqlError::Binding)?;
        let null = arguments.values().first().is_some_and(Value::is_null);
        let mut text = column.quoted;
        append_sql(&mut text, &[if null { " IS NULL" } else { " = ?" }])?;
        Ok(Self(SqlStatement {
            text,
            arguments: if null {
                TursoArguments::default()
            } else {
                arguments
            },
            retention: StatementRetention::default(),
        }))
    }
    /// Use an explicit trusted predicate fragment whose placeholders already carry bindings.
    pub fn trusted(statement: SqlStatement) -> Self {
        Self(statement)
    }
    /// Combine predicates while preserving native parameter ordering.
    #[expect(clippy::result_large_err, reason = "retains the owned SQL error representation without an extra heap allocation")]
    pub fn and(self, other: Self) -> Result<Self, SqlError> {
        Ok(Self(sql! {
            text("(");
            fragment(self.0);
            text(") AND (");
            fragment(other.0);
            text(")");
        }?))
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
            seen: HashSet::new(),
        }
    }
    /// Move one assignment into the description; native encoders retain original value types.
    #[expect(clippy::result_large_err, reason = "retains the owned SQL error representation without an extra heap allocation")]
    pub fn value<T>(mut self, column: impl AsRef<str>, value: T) -> Self
    where
        T: Type<Turso>,
        for<'q> T: Encode<'q, Turso>,
    {
        if self.table.is_err() {
            return self;
        }
        let result = (|| {
            let column = Identifier::new(column)?;
            if self.seen.contains(column.quoted()) {
                return Err(SqlError::Configuration("duplicate mutation column".into()));
            }
            self.seen.try_reserve(1)
                .map_err(|_| SqlError::Memory(MemoryError::CollectionReservationFailed))?;
            self.columns.try_reserve(1)
                .map_err(|_| SqlError::Memory(MemoryError::CollectionReservationFailed))?;
            let mut key = String::new();
            append_sql(&mut key, &[column.quoted()])?;
            self.arguments.add(value).map_err(SqlError::Binding)?;
            self.seen.insert(key);
            self.columns.push(column);
            Ok(())
        })();
        if let Err(error) = result {
            self.table = Err(error);
            drop(std::mem::take(&mut self.arguments));
            drop(std::mem::take(&mut self.columns));
            drop(std::mem::take(&mut self.seen));
        }
        self
    }
    /// Finish an INSERT; at least one unique column/value is required.
    #[expect(clippy::result_large_err, reason = "retains the owned SQL error representation without an extra heap allocation")]
    pub fn insert(self) -> Result<SqlStatement, SqlError> {
        let table = self.table?;
        if self.columns.is_empty() {
            return Err(SqlError::Configuration("insert requires values".into()));
        }
        let mut text = String::new();
        append_sql(&mut text, &["INSERT INTO ", table.quoted(), " ("])?;
        for (index, column) in self.columns.iter().enumerate() {
            append_sql(&mut text, &[if index == 0 { "" } else { ", " }, column.quoted()])?;
        }
        append_sql(&mut text, &[") VALUES (?"])?;
        for _ in 1..self.columns.len() {
            append_sql(&mut text, &[", ?"])?;
        }
        append_sql(&mut text, &[")"])?;
        Ok(SqlStatement {
            text,
            arguments: self.arguments,
            retention: StatementRetention::default(),
        })
    }
    /// Finish a guarded UPDATE; this API requires one predicate rather than implicit full-table writes.
    #[expect(clippy::result_large_err, reason = "retains the owned SQL error representation without an extra heap allocation")]
    pub fn update(mut self, predicate: SqlPredicate) -> Result<SqlStatement, SqlError> {
        let table = self.table?;
        if self.columns.is_empty() {
            return Err(SqlError::Configuration(
                "update requires assignments".into(),
            ));
        }
        let mut text = String::new();
        append_sql(&mut text, &["UPDATE ", table.quoted(), " SET "])?;
        for (index, column) in self.columns.iter().enumerate() {
            append_sql(&mut text, &[if index == 0 { "" } else { ", " }, column.quoted(), " = ?"])?;
        }
        append_sql(&mut text, &[" WHERE ", &predicate.0.text])?;
        self.arguments.try_reserve_exact(predicate.0.arguments.len())
            .map_err(|_| SqlError::Memory(MemoryError::CollectionReservationFailed))?;
        self.arguments.append(predicate.0.arguments);
        Ok(SqlStatement {
            text,
            arguments: self.arguments,
            retention: StatementRetention { prepared_source: None, charges: predicate.0.retention.charges },
        })
    }
}

#[expect(clippy::result_large_err, reason = "retains the original SQL error representation without heap allocation")]
fn append_sql(text: &mut String, parts: &[&str]) -> Result<(), SqlError> {
    let len = parts.iter().try_fold(0usize, |len, part| len.checked_add(part.len()))
        .ok_or(SqlError::Memory(MemoryError::ArithmeticOverflow))?;
    text.try_reserve(len).map_err(|_| SqlError::Memory(MemoryError::CollectionReservationFailed))?;
    for part in parts {
        text.push_str(part);
    }
    Ok(())
}

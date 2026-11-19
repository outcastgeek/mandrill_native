use std::{fmt, sync::Arc};

use sqlx_core::type_info::TypeInfo;

/// SQLite-compatible Turso type information
#[derive(Clone)]
pub struct TursoTypeInfo {
    name: TypeName,
}

#[derive(Clone)]
enum TypeName {
    Static(&'static str),
    Shared(Arc<str>),
}

impl TypeName {
    fn as_str(&self) -> &str {
        match self {
            Self::Static(name) => name,
            Self::Shared(name) => name,
        }
    }
}

impl fmt::Debug for TursoTypeInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TursoTypeInfo")
            .field("name", &self.name())
            .finish()
    }
}

impl PartialEq for TursoTypeInfo {
    fn eq(&self, other: &Self) -> bool {
        self.name() == other.name()
    }
}

impl Eq for TursoTypeInfo {}

#[cfg(feature = "offline")]
impl serde::Serialize for TursoTypeInfo {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("TursoTypeInfo", 1)?;
        state.serialize_field("name", self.name())?;
        state.end()
    }
}

#[cfg(feature = "offline")]
impl<'de> serde::Deserialize<'de> for TursoTypeInfo {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        #[serde(rename = "TursoTypeInfo")]
        struct Fields {
            name: String,
        }
        let fields = Fields::deserialize(deserializer)?;
        Ok(Self::from_name(fields.name))
    }
}

impl TursoTypeInfo {
    /// NULL storage class
    pub const NULL: Self = Self {
        name: TypeName::Static("NULL"),
    };

    /// Creates a type info value from a static type name
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self {
            name: TypeName::Static(name),
        }
    }

    /// Creates a type info value from a dynamic type name
    pub fn from_name(name: impl Into<String>) -> Self {
        Self {
            name: TypeName::Shared(Arc::from(name.into())),
        }
    }

    fn contains_ascii(&self, needle: &str) -> bool {
        self.name()
            .as_bytes()
            .windows(needle.len())
            .any(|part| part.eq_ignore_ascii_case(needle.as_bytes()))
    }

    pub(crate) fn has_integer_affinity(&self) -> bool {
        self.contains_ascii("int")
    }

    pub(crate) fn has_bool_affinity(&self) -> bool {
        self.name().eq_ignore_ascii_case("boolean") || self.name().eq_ignore_ascii_case("bool")
    }

    pub(crate) fn has_text_affinity(&self) -> bool {
        self.contains_ascii("char") || self.contains_ascii("clob") || self.contains_ascii("text")
    }

    pub(crate) fn has_blob_affinity(&self) -> bool {
        self.name().eq_ignore_ascii_case("blob")
    }

    pub(crate) fn has_real_affinity(&self) -> bool {
        self.contains_ascii("real") || self.contains_ascii("floa") || self.contains_ascii("doub")
    }

    pub(crate) fn has_numeric_affinity(&self) -> bool {
        self.contains_ascii("numeric") || self.contains_ascii("decimal")
    }

    #[cfg(any(feature = "chrono", feature = "time"))]
    pub(crate) fn has_date_affinity(&self) -> bool {
        self.name().eq_ignore_ascii_case("date")
    }

    #[cfg(any(feature = "chrono", feature = "time"))]
    pub(crate) fn has_time_affinity(&self) -> bool {
        self.name().eq_ignore_ascii_case("time")
    }

    #[cfg(any(feature = "chrono", feature = "time"))]
    pub(crate) fn has_datetime_affinity(&self) -> bool {
        self.name().eq_ignore_ascii_case("datetime")
            || self.name().eq_ignore_ascii_case("timestamp")
    }
}

impl TypeInfo for TursoTypeInfo {
    fn is_null(&self) -> bool {
        self.name() == "NULL"
    }

    fn name(&self) -> &str {
        self.name.as_str()
    }
}

impl fmt::Display for TursoTypeInfo {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

impl Default for TursoTypeInfo {
    fn default() -> Self {
        Self::NULL
    }
}

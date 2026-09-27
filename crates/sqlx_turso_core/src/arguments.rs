use std::fmt::{self, Write};

use sqlx_core::{
    arguments::Arguments,
    encode::{Encode, IsNull},
    error::BoxDynError,
    impl_into_arguments_for_arguments,
    types::Type,
};

use crate::{Turso, TursoValue};

/// SQLx argument buffer for Turso queries
#[derive(Debug, Default)]
pub struct TursoArguments {
    values: Vec<TursoValue>,
}

impl TursoArguments {
    /// Returns the buffered argument values
    #[must_use]
    pub fn values(&self) -> &[TursoValue] {
        &self.values
    }

    pub(crate) fn into_turso_values(self) -> Vec<turso::Value> {
        self.values
            .into_iter()
            .map(TursoValue::into_turso)
            .collect()
    }
}

impl Arguments for TursoArguments {
    type Database = Turso;

    fn reserve(&mut self, additional: usize, _size: usize) {
        self.values.reserve(additional);
    }

    fn add<'t, T>(&mut self, value: T) -> Result<(), BoxDynError>
    where
        T: Encode<'t, Self::Database> + Type<Self::Database>,
    {
        let len = self.values.len();

        match value.encode(&mut self.values) {
            Ok(IsNull::Yes) => self.values.push(TursoValue::null()),
            Ok(IsNull::No) => {}
            Err(error) => {
                self.values.truncate(len);
                return Err(error);
            }
        }

        Ok(())
    }

    fn len(&self) -> usize {
        self.values.len()
    }

    fn format_placeholder<W: Write>(&self, writer: &mut W) -> fmt::Result {
        write!(writer, "?{}", self.values.len())
    }
}

impl_into_arguments_for_arguments!(TursoArguments);

#[cfg(test)]
mod tests {
    use sqlx_core::{
        arguments::Arguments,
        encode::{Encode, IsNull},
        error::BoxDynError,
        types::Type,
        value::Value,
    };

    use crate::{Turso, TursoArguments, TursoTypeInfo, TursoValue};

    struct ExplicitNull;

    impl Type<Turso> for ExplicitNull {
        fn type_info() -> TursoTypeInfo {
            TursoTypeInfo::new("INTEGER")
        }
    }

    impl Encode<'_, Turso> for ExplicitNull {
        fn encode_by_ref(&self, _values: &mut Vec<TursoValue>) -> Result<IsNull, BoxDynError> {
            Ok(IsNull::Yes)
        }
    }

    struct PartialFailure;

    impl Type<Turso> for PartialFailure {
        fn type_info() -> TursoTypeInfo {
            TursoTypeInfo::new("INTEGER")
        }
    }

    impl Encode<'_, Turso> for PartialFailure {
        fn encode_by_ref(&self, values: &mut Vec<TursoValue>) -> Result<IsNull, BoxDynError> {
            values.push(TursoValue::integer(999));
            Err(std::io::Error::other("intentional encoder failure").into())
        }
    }

    #[test]
    fn null_values_keep_every_argument_slot_in_order() -> Result<(), BoxDynError> {
        let mut arguments = TursoArguments::default();
        arguments.add(Option::<i64>::None)?;
        arguments.add(7_i64)?;
        arguments.add(ExplicitNull)?;
        arguments.add(Option::<String>::None)?;

        assert_eq!(arguments.len(), 4);
        assert!(arguments.values().first().is_some_and(Value::is_null));
        assert!(arguments.values().get(2).is_some_and(Value::is_null));
        assert!(arguments.values().get(3).is_some_and(Value::is_null));
        Ok(())
    }

    #[test]
    fn encoder_error_restores_the_argument_buffer() -> Result<(), BoxDynError> {
        let mut arguments = TursoArguments::default();
        arguments.add(7_i64)?;

        let error = match arguments.add(PartialFailure) {
            Err(error) => error,
            Ok(()) => {
                return Err(std::io::Error::other("test encoder unexpectedly succeeded").into());
            }
        };
        assert_eq!(error.to_string(), "intentional encoder failure");
        assert_eq!(arguments.len(), 1);

        arguments.add(9_i64)?;
        assert_eq!(
            arguments.into_turso_values(),
            [turso::Value::Integer(7), turso::Value::Integer(9)]
        );
        Ok(())
    }

    #[test]
    fn option_is_encodable_for_every_supported_value_family() {
        fn assert_supported<T>()
        where
            T: Type<Turso>,
            for<'query> Option<T>: Encode<'query, Turso> + Type<Turso>,
        {
        }

        assert_supported::<bool>();
        assert_supported::<i8>();
        assert_supported::<i16>();
        assert_supported::<i32>();
        assert_supported::<i64>();
        assert_supported::<u8>();
        assert_supported::<u16>();
        assert_supported::<u32>();
        assert_supported::<u64>();
        assert_supported::<f32>();
        assert_supported::<f64>();
        assert_supported::<String>();
        assert_supported::<Vec<u8>>();

        #[cfg(feature = "bigdecimal")]
        assert_supported::<sqlx_core::types::BigDecimal>();
        #[cfg(feature = "bit-vec")]
        assert_supported::<sqlx_core::types::BitVec>();
        #[cfg(feature = "bstr")]
        assert_supported::<sqlx_core::types::BString>();
        #[cfg(feature = "chrono")]
        assert_supported::<sqlx_core::types::chrono::NaiveDateTime>();
        #[cfg(feature = "ipnet")]
        assert_supported::<sqlx_core::types::ipnet::IpNet>();
        #[cfg(feature = "ipnetwork")]
        assert_supported::<sqlx_core::types::ipnetwork::IpNetwork>();
        #[cfg(feature = "json")]
        assert_supported::<sqlx_core::types::JsonValue>();
        #[cfg(feature = "mac_address")]
        assert_supported::<sqlx_core::types::mac_address::MacAddress>();
        #[cfg(feature = "rust_decimal")]
        assert_supported::<sqlx_core::types::Decimal>();
        #[cfg(feature = "time")]
        assert_supported::<sqlx_core::types::time::OffsetDateTime>();
        #[cfg(feature = "uuid")]
        assert_supported::<sqlx_core::types::Uuid>();
    }
}

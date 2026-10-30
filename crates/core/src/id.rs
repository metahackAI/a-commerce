//! Prefixed, readable identifiers (`prd_windbreaker`, `cs_1042`).
//!
//! Every entity id is `{prefix}_{suffix}`. The prefix says what the id points at, so a model
//! (or a person) can tell a product id from an order id without context. Seed data uses
//! readable suffixes; ids created at run time use a lowercase ULID.

/// Why a string is not a valid id of the expected type.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdError {
    #[error("expected an id starting with `{expected}_`, got `{got}`")]
    WrongPrefix { expected: &'static str, got: String },
    #[error("id suffix may only contain a-z, 0-9 and `_`: `{0}`")]
    BadSuffix(String),
}

/// Checks `{prefix}_{suffix}` and returns the input unchanged when it is valid.
pub fn validate(prefix: &'static str, s: &str) -> Result<(), IdError> {
    let suffix = s
        .strip_prefix(prefix)
        .and_then(|rest| rest.strip_prefix('_'))
        .ok_or_else(|| IdError::WrongPrefix {
            expected: prefix,
            got: s.to_owned(),
        })?;
    let ok = !suffix.is_empty()
        && suffix
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(IdError::BadSuffix(s.to_owned()))
    }
}

/// Defines a newtype id with a fixed prefix, string serde, and a JSON Schema pattern.
#[macro_export]
macro_rules! define_id {
    ($(#[$meta:meta])* $name:ident, $prefix:literal) => {
        $(#[$meta])*
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            pub const PREFIX: &'static str = $prefix;

            pub fn parse(s: impl Into<String>) -> Result<Self, $crate::id::IdError> {
                let s = s.into();
                $crate::id::validate(Self::PREFIX, &s)?;
                Ok(Self(s))
            }

            /// A new id with a lowercase ULID suffix.
            pub fn generate() -> Self {
                Self(format!("{}_{}", Self::PREFIX, $crate::id::new_suffix()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = $crate::id::IdError;
            fn try_from(s: String) -> Result<Self, Self::Error> {
                Self::parse(s)
            }
        }

        impl From<$name> for String {
            fn from(id: $name) -> String {
                id.0
            }
        }

        impl ::std::fmt::Display for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl ::std::fmt::Debug for $name {
            fn fmt(&self, f: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl ::schemars::JsonSchema for $name {
            fn schema_name() -> ::std::borrow::Cow<'static, str> {
                stringify!($name).into()
            }

            fn inline_schema() -> bool {
                true
            }

            fn json_schema(_: &mut ::schemars::SchemaGenerator) -> ::schemars::Schema {
                ::schemars::json_schema!({
                    "type": "string",
                    "pattern": concat!("^", $prefix, "_[a-z0-9_]+$"),
                    "examples": [concat!($prefix, "_example")]
                })
            }
        }
    };
}

#[doc(hidden)]
pub fn new_suffix() -> String {
    ulid::Ulid::generate().to_string().to_ascii_lowercase()
}

define_id!(
    /// A store. Every store-scoped row carries one.
    TenantId,
    "store"
);

#[cfg(test)]
mod tests {
    use super::*;

    define_id!(ProductId, "prd");

    #[test]
    fn accepts_readable_and_generated_ids() {
        assert!(ProductId::parse("prd_windbreaker").is_ok());
        assert!(ProductId::parse(ProductId::generate().as_str()).is_ok());
    }

    #[test]
    fn rejects_other_prefixes_and_bad_suffixes() {
        assert!(matches!(
            ProductId::parse("ord_1"),
            Err(IdError::WrongPrefix { .. })
        ));
        assert!(matches!(
            ProductId::parse("prd_"),
            Err(IdError::BadSuffix(_))
        ));
        assert!(matches!(
            ProductId::parse("prd_Wind"),
            Err(IdError::BadSuffix(_))
        ));
    }

    #[test]
    fn serializes_as_plain_string() {
        let id = ProductId::parse("prd_windbreaker").unwrap();
        assert_eq!(serde_json::to_string(&id).unwrap(), "\"prd_windbreaker\"");
        assert!(serde_json::from_str::<ProductId>("\"ord_1\"").is_err());
    }
}

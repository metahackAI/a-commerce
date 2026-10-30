//! Money as schema.org `MonetaryAmount`: `{"value": 142.00, "currency": "USD"}`.
//!
//! `value` is a JSON number read and written as an exact decimal (serde_json
//! `arbitrary_precision`), in major units of the currency. Floats never touch money.

use std::fmt;

use rust_decimal::{Decimal, RoundingStrategy};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// ISO 4217 currency code.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Currency([u8; 3]);

impl JsonSchema for Currency {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Currency".into()
    }

    fn inline_schema() -> bool {
        true
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[A-Z]{3}$",
            "description": "ISO 4217 currency code",
            "examples": ["USD"]
        })
    }
}

impl Currency {
    pub const USD: Currency = Currency(*b"USD");
    pub const KRW: Currency = Currency(*b"KRW");

    pub fn parse(s: &str) -> Result<Self, MoneyError> {
        let b = s.as_bytes();
        if b.len() == 3 && b.iter().all(u8::is_ascii_uppercase) {
            Ok(Currency([b[0], b[1], b[2]]))
        } else {
            Err(MoneyError::BadCurrency(s.to_owned()))
        }
    }

    pub fn as_str(&self) -> &str {
        // Only ASCII uppercase bytes are ever stored.
        std::str::from_utf8(&self.0).unwrap_or("XXX")
    }

    /// Digits after the decimal point for amounts in this currency.
    pub fn minor_units(self) -> u32 {
        match &self.0 {
            b"KRW" | b"JPY" => 0,
            _ => 2,
        }
    }
}

impl TryFrom<String> for Currency {
    type Error = MoneyError;
    fn try_from(s: String) -> Result<Self, Self::Error> {
        Currency::parse(&s)
    }
}

impl From<Currency> for String {
    fn from(c: Currency) -> String {
        c.as_str().to_owned()
    }
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum MoneyError {
    #[error("`{0}` is not an ISO 4217 currency code")]
    BadCurrency(String),
    #[error("cannot combine {0} and {1}")]
    CurrencyMismatch(Currency, Currency),
}

/// An amount in one currency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(example = Money::example())]
pub struct Money {
    /// Amount in major units, for example 142.00 dollars.
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64")]
    pub value: Decimal,
    pub currency: Currency,
}

impl Money {
    pub fn new(value: Decimal, currency: Currency) -> Self {
        Self { value, currency }
    }

    pub fn zero(currency: Currency) -> Self {
        Self {
            value: Decimal::ZERO,
            currency,
        }
    }

    /// Rounds half away from zero to the currency's minor unit.
    pub fn rounded(self) -> Self {
        let value = self.value.round_dp_with_strategy(
            self.currency.minor_units(),
            RoundingStrategy::MidpointAwayFromZero,
        );
        Self { value, ..self }
    }

    pub fn checked_add(self, other: Money) -> Result<Money, MoneyError> {
        self.same_currency(other)?;
        Ok(Money {
            value: self.value + other.value,
            ..self
        })
    }

    pub fn checked_sub(self, other: Money) -> Result<Money, MoneyError> {
        self.same_currency(other)?;
        Ok(Money {
            value: self.value - other.value,
            ..self
        })
    }

    /// Multiplies by a plain factor (a rate or a quantity). Not rounded.
    pub fn times(self, factor: Decimal) -> Money {
        Money {
            value: self.value * factor,
            ..self
        }
    }

    fn same_currency(self, other: Money) -> Result<(), MoneyError> {
        if self.currency == other.currency {
            Ok(())
        } else {
            Err(MoneyError::CurrencyMismatch(self.currency, other.currency))
        }
    }

    fn example() -> Money {
        Money {
            value: Decimal::new(14200, 2),
            currency: Currency::USD,
        }
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.rounded().value, self.currency)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
    }

    #[test]
    fn reads_and_writes_exact_json_numbers() {
        let m: Money = serde_json::from_str(r#"{"value": 71.20, "currency": "USD"}"#).unwrap();
        assert_eq!(m, usd("71.20"));
        assert_eq!(
            serde_json::to_string(&m).unwrap(),
            r#"{"value":71.20,"currency":"USD"}"#
        );
    }

    #[test]
    fn rounds_half_away_from_zero_to_minor_units() {
        assert_eq!(
            usd("89").times(Decimal::from_str("0.8").unwrap()).rounded(),
            usd("71.20")
        );
        assert_eq!(usd("10.005").rounded(), usd("10.01"));
        let krw = Money::new(Decimal::from_str("1234.5").unwrap(), Currency::KRW);
        assert_eq!(krw.rounded().value, Decimal::from(1235));
    }

    #[test]
    fn refuses_mixed_currencies() {
        let krw = Money::zero(Currency::KRW);
        assert!(usd("1").checked_add(krw).is_err());
    }

    #[test]
    fn rejects_unknown_fields_and_bad_currency() {
        assert!(serde_json::from_str::<Money>(r#"{"value": 1, "currency": "usd"}"#).is_err());
        assert!(
            serde_json::from_str::<Money>(r#"{"value": 1, "currency": "USD", "x": 1}"#).is_err()
        );
    }
}

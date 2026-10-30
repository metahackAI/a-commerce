//! Business rules as data.
//!
//! The defaults are the files in `policies/` (one per domain, plus `approval`). A store may override any key; the result
//! is one `PolicySet` with a version hash that every preview records, so a run can tell when
//! the rules changed under it. Typed views (one struct per file) are added per domain as the
//! operations that read them are built. Design: `docs/backend/commerce.md` §4.

pub mod approval;

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde_json::Value;
use sha2::{Digest, Sha256};

pub use approval::Approval;

/// The policy files every store has, one per domain plus `approval`, in load order.
pub const FILES: &[&str] = &[
    "catalog",
    "merchandising",
    "search",
    "order",
    "fulfillment",
    "promotion",
    "customer",
    "messaging",
    "community",
    "platform",
    "partner",
    "integration",
    "approval",
];

#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    #[error("policy file `{0}` is missing")]
    Missing(&'static str),
    #[error("policy file `{file}` is not valid YAML: {source}")]
    Yaml {
        file: String,
        source: serde_norway::Error,
    },
    #[error("policy `{file}` does not match its schema: {message}")]
    Schema { file: String, message: String },
    #[error("unknown policy file `{0}`")]
    UnknownFile(String),
}

/// All policy files for one store, after overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct PolicySet {
    files: BTreeMap<String, Value>,
    version: String,
}

impl PolicySet {
    /// Loads the defaults from `(file name, YAML text)` pairs; every file in `FILES` is required.
    pub fn from_defaults<'a>(
        sources: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, PolicyError> {
        let mut files = BTreeMap::new();
        for (name, text) in sources {
            if !FILES.contains(&name) {
                return Err(PolicyError::UnknownFile(name.to_owned()));
            }
            files.insert(name.to_owned(), parse_yaml(name, text)?);
        }
        for name in FILES {
            if !files.contains_key(*name) {
                return Err(PolicyError::Missing(name));
            }
        }
        Ok(Self::with_version(files))
    }

    /// Applies a store's partial overrides (`file name → partial YAML`) on top of these values.
    pub fn with_overrides<'a>(
        &self,
        overrides: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Result<Self, PolicyError> {
        let mut files = self.files.clone();
        for (name, text) in overrides {
            let base = files
                .get_mut(name)
                .ok_or_else(|| PolicyError::UnknownFile(name.to_owned()))?;
            merge(base, parse_yaml(name, text)?);
        }
        Ok(Self::with_version(files))
    }

    /// Hash of the effective rules. Recorded on every preview.
    pub fn version(&self) -> &str {
        &self.version
    }

    pub fn raw(&self, file: &str) -> Option<&Value> {
        self.files.get(file)
    }

    /// A typed view of one file. Fails when the file no longer matches the struct.
    pub fn typed<T: DeserializeOwned>(&self, file: &str) -> Result<T, PolicyError> {
        let value = self
            .files
            .get(file)
            .ok_or_else(|| PolicyError::UnknownFile(file.to_owned()))?;
        serde_json::from_value(value.clone()).map_err(|e| PolicyError::Schema {
            file: file.to_owned(),
            message: e.to_string(),
        })
    }

    fn with_version(files: BTreeMap<String, Value>) -> Self {
        let mut hasher = Sha256::new();
        for (name, value) in &files {
            hasher.update(name.as_bytes());
            hasher.update(value.to_string().as_bytes());
        }
        let digest = hasher.finalize();
        let version = digest.iter().take(6).map(|b| format!("{b:02x}")).collect();
        Self { files, version }
    }
}

fn parse_yaml(name: &str, text: &str) -> Result<Value, PolicyError> {
    serde_norway::from_str(text).map_err(|source| PolicyError::Yaml {
        file: name.to_owned(),
        source,
    })
}

/// Deep merge: maps merge key by key, everything else is replaced.
fn merge(base: &mut Value, over: Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(&k) {
                    Some(existing) => merge(existing, v),
                    None => {
                        b.insert(k, v);
                    }
                }
            }
        }
        (b, o) => *b = o,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn defaults() -> PolicySet {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../policies");
        let texts: Vec<(&str, String)> = FILES
            .iter()
            .map(|f| {
                (
                    *f,
                    std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap(),
                )
            })
            .collect();
        PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str()))).unwrap()
    }

    #[test]
    fn loads_every_default_file() {
        let p = defaults();
        for f in FILES {
            assert!(p.raw(f).is_some(), "{f}");
        }
        assert_eq!(
            p.raw("catalog").unwrap()["variant"]["max_variants_per_group"],
            1000
        );
    }

    #[test]
    fn overrides_merge_deeply_and_change_the_version() {
        let p = defaults();
        let demo = p
            .with_overrides([("promotion", "max_discount:\n  by_tier:\n    vip: 0.30\n")])
            .unwrap();
        let md = &demo.raw("promotion").unwrap()["max_discount"];
        assert_eq!(md["by_tier"]["vip"].to_string(), "0.3");
        assert_eq!(
            md["everyone"],
            p.raw("promotion").unwrap()["max_discount"]["everyone"]
        );
        assert_ne!(demo.version(), p.version());
    }

    #[test]
    fn rejects_unknown_and_missing_files() {
        assert!(matches!(
            PolicySet::from_defaults([("pricing", "a: 1")]),
            Err(PolicyError::UnknownFile(_))
        ));
        assert!(matches!(
            PolicySet::from_defaults([("catalog", "a: 1")]),
            Err(PolicyError::Missing(_))
        ));
    }
}

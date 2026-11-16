//! The fashion mall's catalog (`seed/mall/`): 30 brands in six tiers selling 5,000 products,
//! drawn from `assortment.yml` and `brands.yml` with the taxonomy in `policies/taxonomy/`.
//!
//! Every fact comes from a rule, never from a model: the brand's tier sets the price band, the
//! materials, the origins, the colors and the stock; the material sets the care and the
//! thickness; the type sets the warmth, the water resistance and the heel height; the words of
//! the name set the fit. Each brand draws from its own ChaCha8 stream (seeded with the store's
//! seed and the brand id), so a change to one brand moves no other brand's rows. The text model
//! writes descriptions later, from these facts; until then a product has a one-line description.

mod draw;
mod rows;
mod spec;

#[cfg(test)]
mod tests;

use std::path::Path;

use serde_json::Value;

use crate::SeedRows;

/// Generates the mall's brands, categories and products for the files in `dir` (`seed/mall`),
/// with the taxonomy in `taxonomy` (`policies/taxonomy`), on top of `named` (the store's named
/// rows from [`crate::named::load`]), in an order that satisfies foreign keys.
pub fn generate(named: &SeedRows, dir: &Path, taxonomy: &Path) -> Result<SeedRows, String> {
    let mall = spec::Mall::read(dir, taxonomy)?;
    let store = named_store(named, &mall.store.id)?;
    if !named.iter().any(|(t, r)| {
        t == "catalog.merchant_return_policy"
            && r.get("id").and_then(Value::as_str) == Some(mall.store.return_policy.as_str())
    }) {
        return Err(format!(
            "seed/mall: the named rows have no return policy `{}`",
            mall.store.return_policy
        ));
    }
    let products = draw::products(&mall)?;
    Ok(rows::rows(&mall, &store, &products))
}

/// Whether `dir` holds a mall's files rather than a demo store's generator spec.
pub fn is_mall(dir: &Path) -> bool {
    dir.join("assortment.yml").is_file()
}

/// The store's row in the named rows: its currency and the name of its id.
#[derive(Debug)]
pub(crate) struct NamedStore {
    pub currency: String,
}

fn named_store(named: &SeedRows, id: &str) -> Result<NamedStore, String> {
    let row = named
        .iter()
        .find(|(t, r)| {
            t == "platform.online_store" && r.get("id").and_then(Value::as_str) == Some(id)
        })
        .map(|(_, r)| r)
        .ok_or_else(|| format!("seed/mall: the named rows have no store `{id}`"))?;
    let currency = row
        .get("currency")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("seed/mall: store `{id}` has no currency"))?;
    Ok(NamedStore {
        currency: currency.to_owned(),
    })
}

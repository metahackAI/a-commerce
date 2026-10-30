//! The deterministic demo-seed generator: the bulk rows `seed/demo/generate.yml` adds on top
//! of the named rows (more products, members and the sales history behind the scenario
//! numbers in `docs/scenarios.md` §7).
//!
//! One ChaCha8 stream seeded with `random_seed` is consumed in the order of the spec: catalog
//! (summer outerwear, the other categories, placement), members, then orders day by day. The
//! same named rows and spec give the same rows. Money is a JSON number with two decimals,
//! times are RFC 3339 in the store time zone, and every row carries the store's `tenant_id`.
//! A row only references rows earlier in the named rows or in the output.

mod catalog;
mod members;
mod named;
mod orders;
mod spec;
mod util;

#[cfg(test)]
mod schema_check;
#[cfg(test)]
mod tests;

use crate::SeedRows;

/// Generates the bulk rows for `spec_yaml` (the text of `generate.yml`) on top of `named`
/// (from [`crate::named::load`]), in an order that satisfies foreign keys.
pub fn generate(named: &SeedRows, spec_yaml: &str) -> Result<SeedRows, String> {
    let spec = spec::Spec::parse(spec_yaml)?;
    let known = named::Named::read(named, &spec.store)?;
    for c in &spec.never_touch {
        if !known.customers.contains(c) {
            return Err(format!("never_touch customer `{c}` is not a named person"));
        }
    }
    let mut dice = util::Dice::new(spec.seed);

    let catalog = catalog::generate(&spec, &known, &mut dice)?;
    let members = members::generate(&spec, &known, &mut dice)?;
    let skus = orders::skus(&known, &catalog);
    let orders = orders::generate(&spec, &known, &skus, &members, &mut dice)?;

    let autumn_knit = known
        .promotions
        .iter()
        .find(|p| p.id == spec::prose::AUTUMN_KNIT_PROMOTION);
    let mut out = SeedRows::new();
    catalog::rows(&spec, &known, &catalog, autumn_knit, &mut out)?;
    members::rows(&spec, &known, &members, &mut out);
    orders::rows(&spec, &known, &skus, &members, &orders, &mut out);
    Ok(out)
}

//! Merging a guest cart into a signed-in shopper's cart. The shopper sees one cart: their own
//! lines first, then the lines they added as a guest. The same variant in both becomes one line
//! with the larger quantity (capped at `cart.max_quantity_per_line`); lines beyond
//! `cart.max_lines` stay in the guest cart. The first write after signing in moves the guest
//! lines over for real, exactly as shown.

use serde::{Deserialize, Serialize};

/// A cart line as merging needs it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineRef {
    pub id: String,
    pub variant_id: String,
    pub quantity: u32,
}

/// One line of the merged cart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Merged {
    pub id: String,
    pub variant_id: String,
    pub quantity: u32,
    /// The line lives in the guest cart and moves to the shopper's cart on the next write.
    pub from_guest: bool,
    /// A guest line with the same variant folded into this line.
    pub absorbs: Option<String>,
}

/// What a write does to take over the guest cart, as the merge showed it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub(crate) struct Absorb {
    pub guest_cart_id: String,
    /// Guest lines moved to the shopper's cart, with their new position.
    pub moves: Vec<(String, u32)>,
    /// Guest lines folded into a shopper line: (guest line, shopper line, new quantity).
    pub folds: Vec<(String, String, u32)>,
}

pub(crate) fn merge(
    main: &[LineRef],
    guest: &[LineRef],
    max_quantity: u32,
    max_lines: u32,
) -> Vec<Merged> {
    let mut out: Vec<Merged> = main
        .iter()
        .map(|l| Merged {
            id: l.id.clone(),
            variant_id: l.variant_id.clone(),
            quantity: l.quantity,
            from_guest: false,
            absorbs: None,
        })
        .collect();
    for g in guest {
        if let Some(m) = out
            .iter_mut()
            .find(|m| !m.from_guest && m.variant_id == g.variant_id && m.absorbs.is_none())
        {
            m.quantity = m.quantity.max(g.quantity).min(max_quantity);
            m.absorbs = Some(g.id.clone());
        } else if out.len() < max_lines as usize {
            out.push(Merged {
                id: g.id.clone(),
                variant_id: g.variant_id.clone(),
                quantity: g.quantity.min(max_quantity),
                from_guest: true,
                absorbs: None,
            });
        }
    }
    out
}

/// The moves and folds that make the stored carts look like `merged`. `None` when there is no
/// guest line to take over.
pub(crate) fn absorb(guest_cart_id: Option<&str>, merged: &[Merged]) -> Option<Absorb> {
    let guest_cart_id = guest_cart_id?;
    let mut a = Absorb {
        guest_cart_id: guest_cart_id.to_owned(),
        ..Absorb::default()
    };
    for (n, m) in merged.iter().enumerate() {
        let position = u32::try_from(n + 1).unwrap_or(u32::MAX);
        if m.from_guest {
            a.moves.push((m.id.clone(), position));
        }
        if let Some(g) = &m.absorbs {
            a.folds.push((g.clone(), m.id.clone(), m.quantity));
        }
    }
    (!a.moves.is_empty() || !a.folds.is_empty()).then_some(a)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(id: &str, variant: &str, quantity: u32) -> LineRef {
        LineRef {
            id: id.into(),
            variant_id: variant.into(),
            quantity,
        }
    }

    #[test]
    fn guest_lines_follow_the_shoppers_and_duplicates_fold() {
        let main = [line("cln_a", "var_windbreaker_m_slate", 1)];
        let guest = [
            line("cln_g1", "var_windbreaker_m_slate", 2),
            line("cln_g2", "var_merino_tee_m_navy", 1),
        ];
        let m = merge(&main, &guest, 20, 50);
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].quantity, 2);
        assert_eq!(m[0].absorbs.as_deref(), Some("cln_g1"));
        assert!(m[1].from_guest);
        let a = absorb(Some("cart_guest"), &m).unwrap();
        assert_eq!(a.moves, vec![("cln_g2".to_owned(), 2)]);
        assert_eq!(a.folds, vec![("cln_g1".to_owned(), "cln_a".to_owned(), 2)]);
    }

    #[test]
    fn respects_the_line_and_quantity_limits() {
        let main = [line("cln_a", "var_a", 19)];
        let guest = [line("cln_g1", "var_a", 30), line("cln_g2", "var_b", 1)];
        let m = merge(&main, &guest, 20, 1);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].quantity, 20);
    }

    #[test]
    fn nothing_to_take_over_without_a_guest_cart() {
        let main = [line("cln_a", "var_a", 1)];
        let m = merge(&main, &[], 20, 50);
        assert_eq!(absorb(None, &m), None);
        assert_eq!(absorb(Some("cart_g"), &m), None);
    }
}

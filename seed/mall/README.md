# Fashion mall seed: Arbor Row (draft)

[README](../../README.md)

A second demo store: a fashion mall where 30 brands sell 5,000 products, from luxury houses to SPA chains and boutique sellers. It exists to show the platform with a wide catalog and to measure search on it. [Northwind](../demo/README.md) stays as it is, so its scenes and eval do not move.

> **Draft.** Nothing loads these files yet. They fix the shape (categories, garment types, brands, tiers, colors) for review before any generator code or model run. All brands, people and places are fictional.

## Files

| File | Holds |
|---|---|
| [assortment.yml](assortment.yml) | The store (USD, English), size systems, 66 materials (premium, standard or value, and the categories each fits), and 34 standard categories, each linked to a taxonomy node, with its mid-tier price range and its 420 garment types (unisex, women's or men's) |
| [brands.yml](brands.yml) | Six tiers (luxury, designer, contemporary, mid, SPA, boutique) with their price factor, price ending, materials, origins, colors per product, stock and naming; the rules every product must pass; 16 palettes; and 30 brand profiles (tier, identity, audience, category focus, palette, origin) |

The colors come from [policies/taxonomy/colors.yml](../../policies/taxonomy/colors.yml): each name has one to three color families and a swatch. The attributes and nodes come from [policies/taxonomy/](../../policies/taxonomy/README.md).

## How a product is made

1. **Code, by rule.** For each brand, the generator draws its products from these files with a fixed seed: category by focus, type, gender, material, price, colors, sizes and stock. It also sets the taxonomy attributes in `additional_property` and writes the search text from those values. The same seed gives the same rows.
2. **Rules.** Every product is checked against its tier (`brands.yml` `rules`). A product that fails is dropped.
3. **Text model.** It writes the description and summary from the product's facts only, in the tier's voice. The name comes from the tier's template, not from the model.
4. **Image model.** It draws one photo per product in its first color. The prompt names the color's swatch.
5. **People.** They read the pilot (100 products) before the full run.

## Still to do

- The store's platform rows (store, organization, administrators, roles), categories and brands as seed files.
- The generator for this store, and loading it beside Northwind.
- A gateway for this store on the demo server (`ACOM_STORE_ID`).
- The search question set and its measurement ([docs/catalog-taxonomy.md](../../docs/catalog-taxonomy.md) §4.2).

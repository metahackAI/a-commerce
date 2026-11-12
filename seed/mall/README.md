# Fashion mall seed: Arbor Row

[README](../../README.md)

A second demo store: a fashion mall where 30 brands sell 5,000 products, from luxury houses to SPA chains and boutique sellers. It exists to show the platform with a wide catalog and to measure search on it. [Northwind](../demo/README.md) stays as it is, so its scenes and eval do not move. All brands, people and places are fictional.

`acom-commerce migrate` (and `seed`) loads it when `ACOM_SEED_DIR` names this folder beside Northwind's: `ACOM_SEED_DIR=seed/demo,seed/mall`. The store is `store_arbor_row`; sign in as admin / admin123. Products have their facts, prices, colors, sizes and stock, and a one-line description; the written descriptions and photos come later.

## Files

| File | Holds |
|---|---|
| [01_platform.yml](01_platform.yml) | The store (USD, America/Los_Angeles), its organization, the demo administrator, the five system roles with Northwind's scopes, the demo clock (the same instant as Northwind's), the back-office menu |
| [03_catalog.yml](03_catalog.yml) | The return policy every product has |
| [assortment.yml](assortment.yml) | Seed, product count and release dates; 7 departments and 34 categories (each linked to a taxonomy node, with its mid-tier price range and its 420 garment types: unisex, women's or men's); the materials (class, family, care, thickness, fabric, the categories each fits); the materials each type may be made of; fit words; warmth, water resistance and heel height by type |
| [brands.yml](brands.yml) | Six tiers (luxury, designer, contemporary, mid, SPA, boutique) with their price factor and ending, cost share, materials, origins, colors per product, stock, size rules and name words; the rules every product passes; 16 palettes; and 30 brand profiles (tier, identity, audience, category focus, palette, origin, and for 12 brands their own materials) |

The colors come from [policies/taxonomy/colors.yml](../../policies/taxonomy/colors.yml): each name has one to three color families and a swatch. The attributes each category needs come from [policies/taxonomy/nodes.yml](../../policies/taxonomy/nodes.yml) and [apparel.yml](../../policies/taxonomy/apparel.yml).

## How a product is made

The generator is `acom_testkit::mall` ([crates/testkit/src/mall.rs](../../crates/testkit/src/mall.rs)). Every fact comes from these files by rule; no model writes a fact.

1. **Brand.** Each brand draws from its own random stream (the store's seed and the brand id), so a change to one brand moves no other brand's rows. It splits its products over its categories by its focus and draws one price factor inside its tier's range.
2. **Product.** Gender from the brand's audience; a type its tier can make (dearer tiers draw dearer types more often); a material its tier allows for that type (only the brand's own materials when it lists them); a name word that suits the category, type, material and gender; the price in the type's third of the category range × the brand's factor, ended as the tier says (\$890, \$345, \$168, \$79, \$24.90, \$29); colors from the brand's palette; sizes by gender and tier; stock per variant.
3. **Facts.** `additional_property` holds what the category's taxonomy node needs: product type, gender, age group, material family, care and origin; fit (from the words of the name, else drawn), thickness, fabric, warmth and water resistance for clothing; heel height for heels and flats. Each variant has its color families and pattern, and shoes their foot length. `keywords` add the gender, color families, material family and fit.
4. **Checks.** The tests in `crates/testkit/src/mall/tests.rs` check every product against its tier: price band and ending, materials, origins, colors, sizes and stock, a dearer type never cheaper within a brand, dearer tiers dearer overall, unique names per brand, words that suit their product, every attribute its node needs, and every row against the migrations.

Later steps, not built yet:

5. **Text model.** It writes the description and summary from the product's facts only, in the tier's voice (`voice` in brands.yml). The name stays the generator's.
6. **Image model.** It draws one photo per product in its first color. The prompt names the color's swatch.
7. **People.** They read a pilot of 100 products before the full run.

## Numbers

5,000 products and about 106,600 variants with offers, 30 brands, 41 categories (7 departments and 34 categories): 223,363 rows, loaded in about 8 seconds after Northwind.

## Still to do

- A gateway (and web) for this store on the demo server (`ACOM_STORE_ID=store_arbor_row`), and `seed/mall` in the deployed `ACOM_SEED_DIR`.
- Descriptions and photos (steps 5 to 7).
- The search question set and its measurement ([docs/catalog-taxonomy.md](../../docs/catalog-taxonomy.md) §4.2).

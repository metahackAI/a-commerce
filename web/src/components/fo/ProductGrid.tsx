'use client';

// A grid of products from search_products / get_product, reflowing to one column on a phone.

import {Grid} from '@astryxdesign/core/Grid';

import {imageUrl} from '@/app/(storefront)/_lib/format';
import type {ProductSummary} from '@/app/(storefront)/_lib/types';

import {ProductCard} from './ProductCard';

/** The stock a shopper should see for their own size is in the variants; the card shows the product's. */
export function ProductGrid({products, minWidth = 220}: {products: ProductSummary[]; minWidth?: number}) {
  return (
    <Grid columns={{minWidth}} gap={5}>
      {products.map(p => (
        <ProductCard
          key={p.id}
          productId={p.id}
          name={p.name}
          price={p.price}
          currency={p.price_currency}
          member={p.member_price}
          image={imageUrl(p.image)}
          availability={p.availability}
        />
      ))}
    </Grid>
  );
}

'use client';

// look_gallery: the same results with bigger photos and less text (chat.md §3.3, scene F6).

import {Grid} from '@astryxdesign/core/Grid';

import {imageUrl} from '@/app/(storefront)/_lib/format';
import type {ImageRef} from '@/app/(storefront)/_lib/types';
import {ProductCard} from '@/components/fo/ProductCard';

import type {BlockProps} from '../types';
import {BlockFrame, dataOf, intentsFor, list} from './common';
import type {LookGalleryData} from './data';
import {cardPrice} from './values';

export function LookGalleryBlock({block}: BlockProps) {
  const items = list(dataOf<LookGalleryData>(block).items);
  return (
    <BlockFrame block={block}>
      <Grid columns={{minWidth: 200}} gap={4}>
        {items.map(item => (
          <ProductCard
            key={item.product_id}
            productId={item.product_id}
            name={item.name}
            {...cardPrice(item.price, item.list_price)}
            image={imageUrl(item.image as ImageRef)}
            caption={item.caption}
            blockId={block.block_id}
            intents={intentsFor(block, 'product_card', item.product_id)}
          />
        ))}
      </Grid>
    </BlockFrame>
  );
}

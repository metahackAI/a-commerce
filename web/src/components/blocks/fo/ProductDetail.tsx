'use client';

// product_detail: one product the shopper asked about (chat.md §3.3). The block names the product;
// everything about it is read fresh, the same as its page. A storefront block in the back office
// has no cart or shopper around it, so there it stays a card that links to the product.

import {Card} from '@astryxdesign/core/Card';
import {HStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {Text} from '@astryxdesign/core/Text';
import {Thumbnail} from '@astryxdesign/core/Thumbnail';

import {imageUrl} from '@/app/(storefront)/_lib/format';
import type {ImageRef} from '@/app/(storefront)/_lib/types';
import {ProductView} from '@/components/fo/ProductView';
import {useStorefrontIfAny} from '@/components/fo/StorefrontContext';

import type {BlockProps} from '../types';
import {dataOf} from './common';
import type {ProductDetailData} from './data';

export function ProductDetailBlock({block}: BlockProps) {
  const data = dataOf<ProductDetailData>(block);
  const storefront = useStorefrontIfAny();
  if (!data.product_id || !data.name) return null;
  if (storefront) return <ProductView productId={data.product_id} blockId={block.block_id} />;
  return (
    <Card>
      <HStack gap={3} vAlign="center">
        <Thumbnail src={imageUrl(data.image as ImageRef) ?? undefined} alt={data.name} label={data.name} />
        <Link href={`/p/${encodeURIComponent(data.product_id)}`}>
          <Text type="body" weight="semibold">
            {data.name}
          </Text>
        </Link>
      </HStack>
    </Card>
  );
}

'use client';

// A price exactly as the API returns it: the list price, and the shopper's member or limited-time
// price when there is one (search.md, catalog.md). Registered as an AI target so a shopper can ask
// "why this price?".

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';

import {useAiTarget} from '@/ai/useAiTarget';
import {price as fmt} from '@/app/(storefront)/_lib/format';
import type {MemberPrice} from '@/app/(storefront)/_lib/types';

export interface PriceTagProps {
  productId: string;
  name: string;
  price: number;
  currency: string;
  /** `member_price` from get_product / search_products, or a bare number from compare_products. */
  member?: MemberPrice | number | null;
  memberLabel?: string | null;
  size?: 'sm' | 'lg';
  blockId?: string;
}

export function PriceTag({productId, name, price, currency, member, memberLabel, size = 'sm', blockId}: PriceTagProps) {
  const memberPrice = typeof member === 'number' ? member : (member?.price ?? null);
  const label = memberLabel ?? (typeof member === 'object' && member ? member.name : null);
  const shown = memberPrice ?? price;
  const target = useAiTarget({
    kind: 'price',
    label: `${name} · ${fmt(shown, currency)}${label ? ` (${label})` : ''}`,
    ids: {product_id: productId},
    term: 'schema:Offer',
    blockId,
  });
  return (
    <VStack gap={0.5} ref={target.ref} {...target.props}>
      <HStack gap={2} vAlign="center" wrap="wrap">
        <Text type={size === 'lg' ? 'large' : 'body'} weight="semibold" hasTabularNumbers>
          {fmt(shown, currency)}
        </Text>
        {memberPrice !== null && memberPrice !== price ? (
          <Text type="supporting" color="secondary" hasStrikethrough hasTabularNumbers>
            {fmt(price, currency)}
          </Text>
        ) : null}
      </HStack>
      {memberPrice !== null && label ? (
        <Text type="supporting" color="secondary">
          {label} price
        </Text>
      ) : null}
    </VStack>
  );
}

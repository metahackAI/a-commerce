'use client';

// One product in a grid (home, search, category, wishlist, look_gallery). The card is an AI target
// (`product_card`, schema:Product): a shopper can ask "will this be fine in the rain?" or press a
// suggested sentence the agent attached to it.

import {Badge} from '@astryxdesign/core/Badge';
import {Button} from '@astryxdesign/core/Button';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {Text} from '@astryxdesign/core/Text';

import {useAiTarget} from '@/ai/useAiTarget';
import {words} from '@/app/(storefront)/_lib/format';
import type {Availability, MemberPrice} from '@/app/(storefront)/_lib/types';

import {AskButton, Intents} from './Ask';
import {PriceTag} from './PriceTag';
import {ProductImage} from './ProductImage';
import {COMPARE_MAX, useStorefront} from './StorefrontContext';

export interface ProductCardProps {
  productId: string;
  name: string;
  price?: number | null;
  currency?: string;
  member?: MemberPrice | number | null;
  memberLabel?: string | null;
  /** A price the agent already formatted (blocks), used when there is no number. */
  priceText?: string | null;
  listPriceText?: string | null;
  image?: string | null;
  caption?: string | null;
  availability?: Availability | null;
  blockId?: string;
  intents?: string[];
  /** Show the "Compare" toggle (compare_products takes 2–6 products). */
  canCompare?: boolean;
  ratio?: number;
}

export function ProductCard({
  productId,
  name,
  price,
  currency = 'USD',
  member,
  memberLabel,
  priceText,
  listPriceText,
  image,
  caption,
  availability,
  blockId,
  intents = [],
  canCompare = true,
  ratio,
}: ProductCardProps) {
  const target = useAiTarget({
    kind: 'product_card',
    label: name,
    ids: {product_id: productId},
    term: 'schema:Product',
    blockId,
  });
  const {compare, toggleCompare} = useStorefront();
  const comparing = compare.some(c => c.productId === productId);
  const href = `/p/${encodeURIComponent(productId)}`;

  return (
    <VStack gap={2} ref={target.ref} {...target.props}>
      <Link href={href} label={name}>
        <ProductImage src={image} alt={name} ratio={ratio} />
      </Link>
      <VStack gap={1}>
        <HStack gap={1} vAlign="start" justify="between">
          <Link href={href}>
            <Text type="body" weight="semibold" maxLines={2}>
              {name}
            </Text>
          </Link>
          <AskButton target={target} label={name} />
        </HStack>
        {typeof price === 'number' ? (
          <PriceTag productId={productId} name={name} price={price} currency={currency} member={member} memberLabel={memberLabel} blockId={blockId} />
        ) : priceText ? (
          <HStack gap={2} vAlign="center">
            <Text type="body" weight="semibold">
              {priceText}
            </Text>
            {listPriceText ? (
              <Text type="supporting" color="secondary" hasStrikethrough>
                {listPriceText}
              </Text>
            ) : null}
          </HStack>
        ) : null}
        {caption ? (
          <Text type="supporting" color="secondary" maxLines={2}>
            {caption}
          </Text>
        ) : null}
        {availability && availability !== 'in_stock' ? (
          <HStack>
            <Badge label={words(availability)} variant={availability === 'out_of_stock' ? 'warning' : 'neutral'} />
          </HStack>
        ) : null}
        {canCompare ? (
          <HStack>
            <Button
              label={comparing ? 'Comparing' : 'Compare'}
              size="sm"
              variant={comparing ? 'secondary' : 'ghost'}
              isDisabled={!comparing && compare.length >= COMPARE_MAX}
              tooltip={!comparing && compare.length >= COMPARE_MAX ? `Up to ${COMPARE_MAX} products` : undefined}
              onClick={() => toggleCompare({productId, name})}
            />
          </HStack>
        ) : null}
        <Intents says={intents} />
      </VStack>
    </VStack>
  );
}

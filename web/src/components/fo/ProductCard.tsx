'use client';

// One product in a grid (home, search, category, wishlist, look_gallery). The card is an AI target
// (`product_card`, schema:Product): a shopper can ask "will this be fine in the rain?" or press a
// suggested sentence the agent attached to it.

import {Button} from '@astryxdesign/core/Button';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {Text} from '@astryxdesign/core/Text';

import {useAiTarget} from '@/ai/useAiTarget';
import {words} from '@/app/(storefront)/_lib/format';
import type {Availability, MemberPrice} from '@/app/(storefront)/_lib/types';

import {AskButton, Intents} from './Ask';
import {OpenInNewTab, useCanvasLink} from './CanvasPages';
import {PriceTag} from './PriceTag';
import {ProductImage} from './ProductImage';
import {productHref} from './ProductThumb';
import {COMPARE_MAX, useStorefront} from './StorefrontContext';
import {StatusTag} from './StatusTag';

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
  /** The color the picture shows, when the shopper filtered by color: the product opens on it. */
  color?: string | null;
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
  color,
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
  const href = productHref(productId, color);
  const link = useCanvasLink(href, name);

  return (
    <VStack gap={3} ref={target.ref} {...target.props}>
      <ProductImage src={image} alt={color ? `${name}, ${color}` : name} ratio={ratio} {...link} />
      <VStack gap={1.5}>
        <Link {...link}>
          <Text type="body" weight="semibold" maxLines={2}>
            {name}
          </Text>
        </Link>
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
            <StatusTag label={words(availability)} tone={availability === 'out_of_stock' ? 'warning' : 'neutral'} />
          </HStack>
        ) : null}
      </VStack>
      <HStack gap={1} vAlign="center" wrap="wrap">
        <AskButton target={target} label={name} />
        {canCompare ? (
          <Button
            label={comparing ? 'Comparing' : 'Compare'}
            size="sm"
            variant={comparing ? 'secondary' : 'ghost'}
            isDisabled={!comparing && compare.length >= COMPARE_MAX}
            tooltip={!comparing && compare.length >= COMPARE_MAX ? `Up to ${COMPARE_MAX} products` : undefined}
            onClick={() => toggleCompare({productId, name})}
          />
        ) : null}
        <OpenInNewTab href={href} />
      </HStack>
      <Intents says={intents} about={target.context} />
    </VStack>
  );
}

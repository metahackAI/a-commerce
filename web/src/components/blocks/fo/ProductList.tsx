'use client';

// product_list: results as rows with the facts that matter (chat.md §3.3). Each row is a
// product_card target, so "less text, bigger photos" or "compare these" can point at it.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {List, ListItem} from '@astryxdesign/core/List';
import {Text} from '@astryxdesign/core/Text';

import type {UiBlock} from '@/ai/types';
import {useAiTarget} from '@/ai/useAiTarget';
import {imageUrl} from '@/app/(storefront)/_lib/format';
import type {ImageRef} from '@/app/(storefront)/_lib/types';
import {AskButton, Intents} from '@/components/fo/Ask';
import {OpenInNewTab, useCanvasLink} from '@/components/fo/CanvasPages';
import {PriceTag} from '@/components/fo/PriceTag';
import {ProductImage} from '@/components/fo/ProductImage';

import type {BlockProps} from '../types';
import {BlockFrame, dataOf, intentsFor, list} from './common';
import type {ProductListData} from './data';
import {cardPrice} from './values';

type Item = ProductListData['items'][number];

function Row({block, item}: {block: UiBlock; item: Item}) {
  const target = useAiTarget({
    kind: 'product_card',
    label: item.name,
    ids: {product_id: item.product_id},
    term: 'schema:Product',
    blockId: block.block_id,
  });
  const href = `/p/${encodeURIComponent(item.product_id)}`;
  const link = useCanvasLink(href, item.name);
  const p = cardPrice(item.price, item.list_price);
  const facts = list(item.facts).join(' · ');
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      align="start"
      startContent={
        <div style={{width: 88}}>
          <ProductImage src={imageUrl(item.image as ImageRef)} alt={item.name} {...link} />
        </div>
      }
      label={
        <Link {...link}>
          <Text type="body" weight="semibold">
            {item.name}
          </Text>
        </Link>
      }
      description={
        <VStack gap={2}>
          {'price' in p && typeof p.price === 'number' ? (
            <PriceTag
              productId={item.product_id}
              name={item.name}
              price={p.price}
              currency={p.currency}
              member={p.member ?? null}
              memberLabel={p.memberLabel ?? null}
              blockId={block.block_id}
            />
          ) : 'priceText' in p && p.priceText ? (
            <HStack gap={2}>
              <Text type="body" weight="semibold">
                {p.priceText}
              </Text>
              {p.listPriceText ? (
                <Text type="supporting" color="secondary" hasStrikethrough>
                  {p.listPriceText}
                </Text>
              ) : null}
            </HStack>
          ) : null}
          {facts ? (
            <Text type="supporting" color="secondary">
              {facts}
            </Text>
          ) : null}
          <Intents says={intentsFor(block, 'product_card', item.product_id)} about={target.context} />
        </VStack>
      }
      endContent={
        <HStack gap={1} vAlign="center">
          <AskButton target={target} label={item.name} />
          <OpenInNewTab href={href} />
        </HStack>
      }
    />
  );
}

export function ProductListBlock({block}: BlockProps) {
  const items = list(dataOf<ProductListData>(block).items);
  return (
    <BlockFrame block={block}>
      <List hasDividers edge="flush">
        {items.map(item => (
          <Row key={item.product_id} block={block} item={item} />
        ))}
      </List>
    </BlockFrame>
  );
}

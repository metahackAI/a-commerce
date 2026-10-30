'use client';

// cart: the shopper's cart as the assistant last read it (chat.md §3.3, get_cart). Totals are the
// server's (calculate_benefits); changing a quantity is a sentence to the assistant or the cart
// page, never arithmetic here.

import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Button} from '@astryxdesign/core/Button';
import {HStack} from '@astryxdesign/core/Layout';

import {day} from '@/app/(storefront)/_lib/format';

import type {BlockProps} from '../types';
import {BlockActions, BlockFrame, dataOf, list} from './common';
import type {CartBlockData} from './data';
import {Lines} from './Lines';
import {Totals} from './Totals';

export function CartBlock({block}: BlockProps) {
  const data = dataOf<CartBlockData>(block);
  const lines = list(data.lines).map(l => ({
    id: l.cart_line_id,
    product_id: l.product_id,
    name: l.name,
    color: l.color,
    size: l.size,
    quantity: l.quantity,
    price: l.price,
    note: l.note,
    image: l.image,
  }));
  const hasOpen = block.actions.some(a => a.kind === 'open');
  return (
    <BlockFrame
      block={block}
      footer={
        <HStack gap={2} wrap="wrap" vAlign="center">
          <BlockActions block={block} />
          {hasOpen ? null : <Button label="Open cart" size="sm" variant="ghost" href="/cart" />}
        </HStack>
      }>
      <Lines block={block} lines={lines} kind="cart_line" />
      <Totals blockId={block.block_id} subtotal={data.subtotal} discounts={data.discounts} shipping={data.shipping} total={data.total} />
      {data.delivery || data.payment ? (
        <MetadataList>
          {data.delivery?.address_label ? <MetadataListItem label="Ship to">{data.delivery.address_label}</MetadataListItem> : null}
          {data.delivery?.arrives ? <MetadataListItem label="Arrives">{day(data.delivery.arrives)}</MetadataListItem> : null}
          {data.payment?.label ? <MetadataListItem label="Pay with">{data.payment.label}</MetadataListItem> : null}
        </MetadataList>
      ) : null}
    </BlockFrame>
  );
}

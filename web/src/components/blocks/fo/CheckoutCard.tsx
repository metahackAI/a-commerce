'use client';

// checkout_card: an order prepared by prepare_order, exactly as the server priced it (chat.md
// §3.3). Ailism can prepare it; only the shopper places it, with the person-only "Place order"
// button (POST /v1/sales/orders {preview_id}). The agent cannot press it.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Text} from '@astryxdesign/core/Text';
import {useRouter} from 'next/navigation';

import type {BlockAction} from '@/ai/types';
import {dateTime, day} from '@/app/(storefront)/_lib/format';

import type {BlockProps} from '../types';
import {ActionButton, BlockActions, BlockFrame, dataOf, list} from './common';
import type {CheckoutCardData} from './data';
import {Lines} from './Lines';
import {Totals} from './Totals';
import {moneyText} from './values';

export function CheckoutCardBlock({block}: BlockProps) {
  const router = useRouter();
  const data = dataOf<CheckoutCardData>(block);
  const total = moneyText(data.total);
  const payWith = data.payment?.label ?? 'your saved payment method';

  // The block's own person button; without one, the same person-only request from preview_id.
  const own = block.actions.find(a => a.kind === 'person');
  const place: BlockAction | null =
    own ??
    (data.preview_id
      ? {
          kind: 'person',
          action_id: 'place_order',
          label: total ? `Place order · ${total}` : 'Place order',
          method: 'POST',
          path: '/v1/sales/orders',
          body: {preview_id: data.preview_id},
          confirm: total ? `Place this order and pay ${total} with ${payWith}?` : `Place this order and pay with ${payWith}?`,
        }
      : null);
  const others = {...block, actions: block.actions.filter(a => a !== own)};

  const lines = list(data.lines).map((l, i) => ({
    id: l.cart_line_id ?? l.variant_id ?? `${i}`,
    product_id: l.product_id,
    name: l.name,
    color: l.color,
    size: l.size,
    quantity: l.quantity,
    price: l.price,
    note: l.note,
    image: l.image,
  }));

  return (
    <BlockFrame
      block={block}
      footer={
        <VStack gap={2}>
          {place ? (
            <HStack>
              <ActionButton blockId={block.block_id}
                action={place}
                onDone={result => {
                  const id = (result as {id?: unknown} | null)?.id;
                  if (typeof id === 'string') router.push(`/orders/${encodeURIComponent(id)}?placed=1`);
                }}
              />
            </HStack>
          ) : null}
          <Text type="supporting" color="secondary">
            {data.note && data.note.includes('Only you can place it') ? data.note : `${data.note ? `${data.note} ` : ''}Only you can place it.`}
          </Text>
          <BlockActions block={others} />
        </VStack>
      }>
      <Lines block={block} lines={lines} kind="cart_line" />
      <Totals
        blockId={block.block_id}
        subtotal={data.subtotal}
        discounts={data.discounts}
        shipping={data.shipping}
        extra={data.gift_wrap ? [{label: 'Gift wrap', value: data.gift_wrap}] : undefined}
        total={data.total}
      />
      <MetadataList>
        {data.delivery?.address_label ? <MetadataListItem label="Ship to">{data.delivery.address_label}</MetadataListItem> : null}
        {data.order_type === 'gift' ? (
          <MetadataListItem label="Address">
            {data.recipient?.mode === 'recipient_accepts' ? 'They give it when they accept the gift' : 'Used for this order only, not saved'}
          </MetadataListItem>
        ) : null}
        {data.gift_options?.card_message ? <MetadataListItem label="Card">{`“${data.gift_options.card_message}”`}</MetadataListItem> : null}
        {data.gift_options?.gift_receipt ? (
          <MetadataListItem label="Receipt">{data.gift_options.receipt_hides_prices ? 'Gift receipt, no prices shown' : 'Gift receipt'}</MetadataListItem>
        ) : null}
        {data.gift_options?.exchange_until ? <MetadataListItem label="Exchange">{`Size exchange until ${day(data.gift_options.exchange_until)}`}</MetadataListItem> : null}
        {data.delivery?.arrives ? <MetadataListItem label="Arrives">{day(data.delivery.arrives)}</MetadataListItem> : null}
        {data.payment?.label ? <MetadataListItem label="Pay with">{data.payment.label}</MetadataListItem> : null}
        {data.expires_at ? <MetadataListItem label="Prepared until">{dateTime(data.expires_at)}</MetadataListItem> : null}
      </MetadataList>
    </BlockFrame>
  );
}

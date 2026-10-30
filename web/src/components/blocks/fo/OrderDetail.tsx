'use client';

// order_detail: one order as the assistant read it (chat.md §3.3, get_order): status, lines,
// delivery, payment, totals. Return or exchange questions go back to the assistant (`say`); the
// full order page has the shopper's own cancel and return buttons.

import {Button} from '@astryxdesign/core/Button';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Text} from '@astryxdesign/core/Text';

import {useAiTarget} from '@/ai/useAiTarget';
import {day, dateTime, words} from '@/app/(storefront)/_lib/format';

import type {BlockProps} from '../types';
import {BlockActions, BlockFrame, dataOf, list} from './common';
import type {OrderDetailData} from './data';
import {Lines} from './Lines';
import {Totals} from './Totals';
import {StatusTag} from '@/components/fo/StatusTag';

function statusVariant(status: string): 'success' | 'warning' | 'error' | 'info' | 'neutral' {
  if (status === 'delivered') return 'success';
  if (status === 'problem' || status === 'payment_due') return 'warning';
  if (status === 'cancelled' || status === 'returned') return 'neutral';
  return 'info';
}

export function OrderDetailBlock({block}: BlockProps) {
  const data = dataOf<OrderDetailData>(block);
  const status = data.status ?? '';
  const delivery = data.delivery ?? null;
  const payment = data.payment ?? null;
  const totals = data.totals ?? null;
  const target = useAiTarget({
    kind: 'order_row',
    label: `Order #${data.order_number ?? ''} · ${words(status)}`,
    ids: {order_id: data.order_id ?? data.order_number ?? block.block_id},
    term: 'schema:Order',
    blockId: block.block_id,
  });
  const lines = list(data.lines).map((l, i) => ({
    id: l.order_item_id ?? l.id ?? `${i}`,
    product_id: l.product_id,
    name: l.name,
    size: l.size,
    color: l.color,
    quantity: l.quantity ?? l.order_quantity,
    price: l.price,
    status: l.status,
  }));
  const arrives = delivery?.delivered_at
    ? `Delivered ${dateTime(delivery.delivered_at)}`
    : delivery?.arrives
      ? day(delivery.arrives)
      : delivery?.expected_arrival_until
        ? `By ${dateTime(delivery.expected_arrival_until)}`
        : null;
  const deliveryState = delivery?.delivery_state ?? delivery?.state ?? null;
  const pay = payment?.label ?? (payment?.card_last4 ? `Card ending ${payment.card_last4}` : payment?.payment_method ? words(payment.payment_method) : null);

  return (
    <BlockFrame
      block={block}
      footer={
        <HStack gap={2} wrap="wrap" vAlign="center">
          <BlockActions block={block} />
          {data.order_id && !block.actions.some(a => a.kind === 'open') ? (
            <Button label="Open order" size="sm" variant="ghost" href={`/orders/${encodeURIComponent(data.order_id)}`} />
          ) : null}
        </HStack>
      }>
      <VStack gap={1} ref={target.ref} {...target.props}>
        <HStack gap={2} vAlign="center" wrap="wrap">
          <Text type="body" weight="semibold">
            Order #{data.order_number}
          </Text>
          {status ? <StatusTag label={words(status)} tone={statusVariant(status)} /> : null}
        </HStack>
      </VStack>
      <Lines block={block} lines={lines} kind="order_row" />
      <MetadataList>
        {delivery?.address_label || delivery?.address ? <MetadataListItem label="Ship to">{delivery.address_label ?? delivery.address}</MetadataListItem> : null}
        {deliveryState ? <MetadataListItem label="Delivery">{words(deliveryState)}</MetadataListItem> : null}
        {arrives ? <MetadataListItem label="Arrives">{arrives}</MetadataListItem> : null}
        {delivery?.tracking_number ? (
          <MetadataListItem label="Tracking">{[delivery.carrier, delivery.tracking_number].filter(Boolean).join(' ')}</MetadataListItem>
        ) : null}
        {pay ? <MetadataListItem label="Paid with">{pay}</MetadataListItem> : null}
        {payment?.payment_status ? <MetadataListItem label="Payment">{words(payment.payment_status)}</MetadataListItem> : null}
      </MetadataList>
      {totals ? (
        <Totals blockId={block.block_id} subtotal={totals.subtotal} discounts={totals.discounts} shipping={totals.shipping} total={totals.total} />
      ) : null}
    </BlockFrame>
  );
}

'use client';

// Orders as the shopper sees them: rows of get_my_orders, the lines and totals of get_order, and
// delivery tracking (get_delivery, track_shipment). Each row is an AI target (`order_row`) so the
// shopper can ask "where is it?" or "I want to return this".

import {Badge} from '@astryxdesign/core/Badge';
import {Card} from '@astryxdesign/core/Card';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {List, ListItem} from '@astryxdesign/core/List';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Heading, Text} from '@astryxdesign/core/Text';

import {useAiTarget} from '@/ai/useAiTarget';
import {dateTime, longDate, money, words} from '@/app/(storefront)/_lib/format';
import type {
  Delivery,
  OrderItem,
  OrderStatus,
  OrderSummary,
  PostalAddress,
  ReturnHistoryItem,
  TrackingEvent,
} from '@/app/(storefront)/_lib/types';

import {AskButton} from './Ask';
import {ReturnPanel} from './ReturnPanel';

export function statusVariant(s: string): 'success' | 'warning' | 'error' | 'info' | 'neutral' {
  if (s === 'delivered' || s === 'completed') return 'success';
  if (s === 'problem' || s === 'declined' || s === 'delivery_failed') return 'error';
  if (s === 'payment_due' || s === 'charge_due' || s === 'photos_requested') return 'warning';
  if (s === 'in_transit' || s === 'processing' || s === 'requested' || s === 'approved') return 'info';
  return 'neutral';
}

export function StatusBadge({status}: {status: OrderStatus | string}) {
  return <Badge label={words(status)} variant={statusVariant(status)} />;
}

export function OrderRow({order}: {order: OrderSummary}) {
  const items = order.ordered_item.map(i => `${i.name}${i.size ? ` (${i.size})` : ''}${i.order_quantity > 1 ? ` × ${i.order_quantity}` : ''}`).join(', ');
  const target = useAiTarget({
    kind: 'order_row',
    label: `Order #${order.order_number} · ${words(order.order_status)} · ${money(order.total)}`,
    ids: {order_id: order.id},
    term: 'schema:Order',
  });
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      align="start"
      label={
        <HStack gap={2} vAlign="center" wrap="wrap">
          <Link href={`/orders/${encodeURIComponent(order.id)}`}>
            <Text weight="semibold">Order #{order.order_number}</Text>
          </Link>
          <StatusBadge status={order.order_status} />
        </HStack>
      }
      description={
        <VStack gap={0.5}>
          <Text type="supporting" color="secondary">
            {longDate(order.order_date)}
            {order.delivered_at ? ` · delivered ${longDate(order.delivered_at)}` : ''}
          </Text>
          <Text maxLines={2}>{items}</Text>
        </VStack>
      }
      endContent={
        <HStack gap={1} vAlign="center">
          <Text weight="semibold" hasTabularNumbers>
            {money(order.total)}
          </Text>
          <AskButton target={target} label={`order #${order.order_number}`} />
        </HStack>
      }
    />
  );
}

export function OrderList({orders}: {orders: OrderSummary[]}) {
  return (
    <List hasDividers density="spacious">
      {orders.map(o => (
        <OrderRow key={o.id} order={o} />
      ))}
    </List>
  );
}

function OrderLineRow({orderId, item, canReturn}: {orderId: string; item: OrderItem; canReturn: boolean}) {
  const options = [item.color, item.size ? `size ${item.size}` : null].filter(Boolean).join(' · ');
  const target = useAiTarget({
    kind: 'order_line',
    label: `${item.name}${options ? `, ${options}` : ''} × ${item.order_quantity}`,
    ids: {order_item_id: item.id, order_id: orderId, product_id: item.product_id},
    term: 'schema:OrderItem',
  });
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      align="start"
      label={
        <HStack gap={2} vAlign="center" wrap="wrap">
          <Link href={`/p/${encodeURIComponent(item.product_id)}`}>
            <Text weight="semibold">{item.name}</Text>
          </Link>
          <StatusBadge status={item.order_item_status} />
        </HStack>
      }
      description={
        <VStack gap={2}>
          <Text type="supporting" color="secondary">
            {[options, `Qty ${item.order_quantity}`, item.price ? `${money(item.price)} each` : null, item.discount && item.discount.value > 0 ? `−${money(item.discount)} off` : null]
              .filter(Boolean)
              .join(' · ')}
          </Text>
          {canReturn && item.order_item_status === 'delivered' ? <ReturnPanel orderId={orderId} item={item} /> : null}
        </VStack>
      }
      endContent={<AskButton target={target} label={item.name} />}
    />
  );
}

export function OrderLines({orderId, items}: {orderId: string; items: OrderItem[]}) {
  return (
    <List hasDividers density="spacious">
      {items.map(i => (
        <OrderLineRow key={i.id} orderId={orderId} item={i} canReturn />
      ))}
    </List>
  );
}

function addressText(a: string | PostalAddress | null | undefined): string {
  if (!a) return '';
  if (typeof a === 'string') return a;
  return [a.name, a.street_address, a.address_locality, a.address_region, a.postal_code].filter(Boolean).join(', ');
}

export function DeliveryCard({
  orderId,
  delivery,
  fallback,
  events,
}: {
  orderId: string;
  delivery: Delivery | null;
  /** get_order's delivery summary when get_delivery is not available. */
  fallback: {id?: string; delivery_state?: string | null; expected_arrival_until?: string | null; delivered_at?: string | null; delivery_address?: PostalAddress | null} | null;
  events: TrackingEvent[];
}) {
  const state = delivery?.delivery_state ?? fallback?.delivery_state ?? null;
  const target = useAiTarget({
    kind: 'delivery',
    label: `Delivery for this order${state ? ` · ${words(state)}` : ''}`,
    ids: {delivery_id: delivery?.id ?? fallback?.id ?? orderId, order_id: orderId},
    term: 'schema:ParcelDelivery',
  });
  const expected = delivery?.expected_arrival_until ?? fallback?.expected_arrival_until ?? null;
  const timeline = events.length > 0 ? events : (delivery?.events ?? []).map(e => ({...e, location: null}));
  return (
    <Card ref={target.ref} {...target.props}>
      <VStack gap={3}>
        <HStack gap={2} vAlign="center" justify="between">
          <Heading level={2}>Delivery</Heading>
          <HStack gap={1} vAlign="center">
            {state ? <StatusBadge status={state} /> : null}
            <AskButton target={target} label="this delivery" />
          </HStack>
        </HStack>
        <MetadataList>
          {addressText(delivery?.delivery_address ?? fallback?.delivery_address) ? (
            <MetadataListItem label="To">{addressText(delivery?.delivery_address ?? fallback?.delivery_address)}</MetadataListItem>
          ) : null}
          {fallback?.delivered_at ? <MetadataListItem label="Delivered">{dateTime(fallback.delivered_at)}</MetadataListItem> : null}
          {!fallback?.delivered_at && expected ? <MetadataListItem label="Expected by">{dateTime(expected)}</MetadataListItem> : null}
          {delivery?.carrier || delivery?.tracking_number ? (
            <MetadataListItem label="Tracking">{[delivery.carrier, delivery.tracking_number].filter(Boolean).join(' · ')}</MetadataListItem>
          ) : null}
        </MetadataList>
        {timeline.length > 0 ? (
          <List density="compact">
            {timeline.map((e, i) => (
              <ListItem key={`${e.at}-${i}`} label={words(e.delivery_state)} description={[dateTime(e.at), e.location].filter(Boolean).join(' · ')} />
            ))}
          </List>
        ) : null}
      </VStack>
    </Card>
  );
}

function ReturnRow({item}: {item: ReturnHistoryItem}) {
  const target = useAiTarget({kind: 'claim_row', label: `${words(item.claim_type)} · ${item.item} · ${words(item.status)}`, ids: {claim_id: item.id}});
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      label={
        <HStack gap={2} vAlign="center" wrap="wrap">
          <Text weight="semibold">{item.item}</Text>
          <StatusBadge status={item.status} />
        </HStack>
      }
      description={`${words(item.claim_type)} · order #${item.order_number} · ${words(item.reason)} · requested ${longDate(item.requested_at)}`}
      endContent={
        <Text hasTabularNumbers>
          {money(item.amount)}
        </Text>
      }
    />
  );
}

export function ReturnHistory({items}: {items: ReturnHistoryItem[]}) {
  return (
    <List hasDividers>
      {items.map(i => (
        <ReturnRow key={i.id} item={i} />
      ))}
    </List>
  );
}

import type {Metadata} from 'next';
import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {HStack, Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';

import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {OrderList, ReturnHistory} from '@/components/fo/OrderViews';
import type {Page} from '@/lib/api';

import {words} from '../_lib/format';
import {load, shopper} from '../_lib/server';
import type {ClaimSummary, OrderSummary, ReturnHistoryItem} from '../_lib/types';

export const metadata: Metadata = {title: 'Orders'};

export default async function OrdersPage({searchParams}: {searchParams: Promise<Record<string, string | string[] | undefined>>}) {
  const who = await shopper();
  const {cursor} = await searchParams;
  const after = typeof cursor === 'string' ? cursor : undefined;

  const content = await (async () => {
    if (!who.isShopper) {
      return (
        <Banner
          status="info"
          title="Sign in to see your orders"
          description="Orders, delivery tracking and returns belong to a signed-in shopper."
          endContent={<Button label="Sign in" variant="primary" size="sm" href="/login?next=%2Forders" />}
        />
      );
    }
    // The shopper's own requests (search_claims answers a shopper only for themselves), named by
    // the order numbers and items of their orders.
    const [orders, claims] = await Promise.all([
      load<Page<OrderSummary>>('/v1/sales/orders/mine', {limit: 20, cursor: after}),
      load<Page<ClaimSummary>>('/v1/fulfillment/claims', {limit: 20}),
    ]);
    const byOrder = new Map((orders.ok ? orders.data.items : []).map(o => [o.id, o]));
    const returns = !claims.ok
      ? claims
      : {
          ok: true as const,
          data: claims.data.items.map((c): ReturnHistoryItem => {
            const o = byOrder.get(c.order_id);
            return {
              id: c.id,
              claim_type: c.claim_type,
              order_id: c.order_id,
              order_number: o?.order_number ?? c.order_id,
              item: o?.ordered_item.map(i => i.name).join(', ') || words(c.claim_type),
              amount: c.amount,
              reason: c.reason ?? '',
              status: c.status,
              requested_at: c.created_at,
            };
          }),
        };
    return (
      <VStack gap={8}>
        {!orders.ok ? (
          <ErrorNotice error={orders.error} askAbout="Where are my orders?" />
        ) : orders.data.items.length === 0 ? (
          <EmptyState title="No orders yet" description="When you place an order it shows up here with its delivery." actions={<Button label="Start shopping" variant="primary" href="/search" />} />
        ) : (
          <VStack gap={3}>
            <OrderList orders={orders.data.items} />
            {after || orders.data.next_cursor ? (
              <HStack gap={2} justify="center">
                {after ? <Button label="Newest orders" variant="ghost" href="/orders" /> : null}
                {orders.data.next_cursor ? <Button label="Older orders" href={`/orders?cursor=${encodeURIComponent(orders.data.next_cursor)}`} /> : null}
              </HStack>
            ) : null}
          </VStack>
        )}
        <VStack gap={3}>
          <Heading level={2}>Returns and cancellations</Heading>
          {!returns.ok ? (
            <Text color="secondary">{returns.error.message}</Text>
          ) : returns.data.length === 0 ? (
            <Text color="secondary">No returns, exchanges or cancellations yet.</Text>
          ) : (
            <ReturnHistory items={returns.data} />
          )}
        </VStack>
      </VStack>
    );
  })();

  return (
    <Layout
      height="auto"
      contentWidth={1000}
      content={
        <LayoutContent padding={6}>
          <VStack gap={4}>
            <Heading level={1}>Your orders</Heading>
            {content}
          </VStack>
        </LayoutContent>
      }
    />
  );
}

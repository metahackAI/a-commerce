import type {Metadata} from 'next';
import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Heading, Text} from '@astryxdesign/core/Text';

import {CancelPanel} from '@/components/fo/CancelPanel';
import {CartSummary} from '@/components/fo/CartSummary';
import {ErrorNotice} from '@/components/fo/ErrorNotice';
import {DeliveryCard, GiftOnOrder, OrderLines, StatusBadge} from '@/components/fo/OrderViews';
import type {Page} from '@/lib/api';

import {longDate, words} from '../../_lib/format';
import {load, shopper} from '../../_lib/server';
import type {Delivery, Order, TrackingEvent} from '../../_lib/types';

type Params = Promise<{order_id: string}>;
type Search = Promise<Record<string, string | string[] | undefined>>;

export async function generateMetadata({params}: {params: Params}): Promise<Metadata> {
  const id = decodeURIComponent((await params).order_id);
  const order = await load<Order>(`/v1/sales/orders/${encodeURIComponent(id)}`, {fields: ['id', 'order_number']});
  return {title: order.ok ? `Order #${order.data.order_number}` : 'Order'};
}

/** Orders that have not shipped yet may still be cancelled; get_cancel_options decides what exactly. */
const BEFORE_SHIPPING = new Set(['payment_due', 'processing']);

export default async function OrderPage({params, searchParams}: {params: Params; searchParams: Search}) {
  const id = decodeURIComponent((await params).order_id);
  const [{placed}, who] = await Promise.all([searchParams, shopper()]);

  const wrap = (body: React.ReactNode) => (
    <Layout
      height="auto"
      contentWidth={1100}
      content={
        <LayoutContent padding={6}>
          <VStack gap={5}>{body}</VStack>
        </LayoutContent>
      }
    />
  );

  if (!who.isShopper) {
    return wrap(
      <Banner
        status="info"
        title="Sign in to see this order"
        endContent={<Button label="Sign in" size="sm" variant="primary" href={`/login?next=${encodeURIComponent(`/orders/${id}`)}`} />}
      />,
    );
  }

  const order = await load<Order>(`/v1/sales/orders/${encodeURIComponent(id)}`);
  if (!order.ok) return wrap(<ErrorNotice error={order.error} title="This order is not available." />);
  const o = order.data;

  const deliveryId = o.delivery?.id ?? null;
  const [delivery, tracking] = await Promise.all([
    deliveryId ? load<Delivery>(`/v1/fulfillment/deliveries/${encodeURIComponent(deliveryId)}`) : null,
    deliveryId
      ? load<Page<TrackingEvent>>(`/v1/fulfillment/deliveries/${encodeURIComponent(deliveryId)}/tracking`, {mode: 'delivery', delivery_id: deliveryId, limit: 20})
      : null,
  ]);

  return wrap(
    <>
      {placed ? (
        <Banner
          status="success"
          title={`Order #${o.order_number} is placed`}
          description="Thank you. You will see delivery updates here."
          endContent={<Button label="Keep shopping" size="sm" href="/" />}
        />
      ) : null}
      <VStack gap={1}>
        <HStack gap={2} vAlign="center" wrap="wrap">
          <Heading level={1}>Order #{o.order_number}</Heading>
          <StatusBadge status={o.order_status} />
        </HStack>
        <Text color="secondary">Placed {longDate(o.order_date)}</Text>
      </VStack>
      <Grid columns={{minWidth: 320, max: 2}} gap={6} align="start">
        <VStack gap={4}>
          <OrderLines orderId={o.id} items={o.ordered_item} claims={o.claims} />
          {BEFORE_SHIPPING.has(o.order_status) ? <CancelPanel orderId={o.id} orderNumber={o.order_number} /> : null}
        </VStack>
        <VStack gap={4}>
          <Card>
            <VStack gap={3}>
              <Heading level={2}>Payment</Heading>
              <CartSummary
                subtotal={o.totals.subtotal}
                discounts={o.totals.discounts}
                shipping={o.totals.shipping}
                giftWrap={o.totals.gift_wrap ?? undefined}
                total={o.totals.total}
                coupons={o.coupons_recorded}
              />
              {o.payment ? (
                <MetadataList>
                  <MetadataListItem label={o.payment.payment_status === 'payment_due' ? 'Pays with' : 'Paid with'}>
                    {[words(o.payment.payment_method), o.payment.card_last4 ? `ending ${o.payment.card_last4}` : null].filter(Boolean).join(' ')}
                  </MetadataListItem>
                  {o.payment.payment_status ? <MetadataListItem label="Status">{words(o.payment.payment_status)}</MetadataListItem> : null}
                </MetadataList>
              ) : null}
            </VStack>
          </Card>
          {o.gift ? <GiftOnOrder orderId={o.id} gift={o.gift} /> : null}
          {o.delivery?.delivery_address || delivery?.ok ? (
            <DeliveryCard
              orderId={o.id}
              delivery={delivery?.ok ? delivery.data : null}
              fallback={o.delivery ?? null}
              events={tracking?.ok ? tracking.data.items : []}
            />
          ) : null}
        </VStack>
      </Grid>
      <HStack>
        <Button label="All orders" variant="ghost" href="/orders" />
      </HStack>
    </>,
  );
}

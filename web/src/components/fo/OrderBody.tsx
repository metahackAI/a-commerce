'use client';

// One order's lines, payment, gift and delivery: on its page and on the canvas.

import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {VStack} from '@astryxdesign/core/Layout';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Heading} from '@astryxdesign/core/Text';

import {words} from '@/app/(storefront)/_lib/format';
import type {Delivery, Order, TrackingEvent} from '@/app/(storefront)/_lib/types';

import {CancelPanel} from './CancelPanel';
import {CartSummary} from './CartSummary';
import {DeliveryCard, GiftOnOrder, OrderLines} from './OrderViews';

/** Orders that have not shipped yet may still be cancelled; get_cancel_options decides what exactly. */
const BEFORE_SHIPPING = new Set(['payment_due', 'processing']);

export function OrderBody({order: o, delivery, tracking}: {order: Order; delivery: Delivery | null; tracking: TrackingEvent[]}) {
  return (
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
        {o.delivery?.delivery_address || delivery ? <DeliveryCard orderId={o.id} delivery={delivery} fallback={o.delivery ?? null} events={tracking} /> : null}
      </VStack>
    </Grid>
  );
}

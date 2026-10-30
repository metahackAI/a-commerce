'use client';

// One order for an operator: lines, totals and recorded coupons (get_order), the payment
// (get_payment), the delivery (get_delivery) and the carrier's tracking events (track_shipment).

import {VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {Button} from '@astryxdesign/core/Button';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';

import type {ApiErrorBody} from '@/lib/api';
import {address, count, dateTime, isMoney, money, words} from '@/app/bo/_lib/format';
import {withParams} from '@/app/bo/_lib/links';
import type {Delivery, Order, Payment, TrackingEvent} from '@/app/bo/_lib/types';

import {ErrorState} from './ErrorState';
import {PageFrame, PageSection} from './PageFrame';
import {StateToken} from './StateToken';
import {DataTable, Figure} from './tables';
import {TargetFact} from './targets';

export interface OrderViewProps {
  order: Order;
  payment: Payment | null;
  paymentError: ApiErrorBody | null;
  delivery: Delivery | null;
  deliveryError: ApiErrorBody | null;
  tracking: TrackingEvent[] | null;
  trackingError: ApiErrorBody | null;
}

export function OrderView({order: o, payment, paymentError, delivery, deliveryError, tracking, trackingError}: OrderViewProps) {
  const ids = {order_id: o.id};
  const t = o.totals;

  return (
    <PageFrame
      back={{href: '/bo/orders', label: 'Orders'}}
      title={`Order #${o.order_number}`}
      meta={
        <>
          <StateToken state={o.order_status} size="md" />
          <Text type="supporting" color="secondary">
            {`${o.id} · placed ${dateTime(o.order_date)}`}
          </Text>
        </>
      }
      actions={
        <>
          <Button label="Customer" href={`/bo/customers/${encodeURIComponent(o.customer_id)}`} />
          <Button label="Activity" variant="ghost" href={withParams('/bo/activity', {object_id: o.id})} />
        </>
      }
      panel={
        <MetadataList title="Payment">
          {paymentError ? (
            <MetadataListItem label="Payment">{paymentError.message}</MetadataListItem>
          ) : payment ? (
            <>
              <MetadataListItem label="Paid with">{payment.label}</MetadataListItem>
              <MetadataListItem label="Status">
                <StateToken state={payment.payment_status} />
              </MetadataListItem>
              <MetadataListItem label="Amount">{money(payment.amount)}</MetadataListItem>
              <MetadataListItem label="Method">{words(payment.payment_method)}</MetadataListItem>
              {payment.installment_months ? <MetadataListItem label="Installments">{`${payment.installment_months} months`}</MetadataListItem> : null}
              <MetadataListItem label="Payment id">{payment.id}</MetadataListItem>
            </>
          ) : o.payment ? (
            <>
              <MetadataListItem label="Paid with">{`${words(o.payment.payment_method)}${o.payment.card_last4 ? ` ending ${o.payment.card_last4}` : ''}`}</MetadataListItem>
              <MetadataListItem label="Status">
                <StateToken state={o.payment.payment_status} />
              </MetadataListItem>
            </>
          ) : (
            <MetadataListItem label="Payment">—</MetadataListItem>
          )}
        </MetadataList>
      }>
      <PageSection title="Items">
        <DataTable
          rows={o.ordered_item}
          rowKey={l => l.id}
          empty={{title: 'No items'}}
          target={l => ({
            kind: 'order_row',
            label: `#${o.order_number} ${l.name} ${[l.size, l.color].filter(Boolean).join(' ')} ×${l.order_quantity} · ${money(l.price)} · ${l.order_item_status}`,
            ids: {order_item_id: l.id, ...ids},
            term: 'schema:OrderItem',
          })}
          columns={[
            {
              header: 'Item',
              cell: l => (
                <VStack gap={0}>
                  <Link href={`/bo/products/${encodeURIComponent(l.product_id)}`}>{l.name}</Link>
                  <Text type="supporting" color="secondary">
                    {[l.size, l.color, l.variant_id].filter(Boolean).join(' · ')}
                  </Text>
                </VStack>
              ),
            },
            {header: 'Qty', cell: l => <Figure>{count(l.order_quantity)}</Figure>},
            {header: 'Price', cell: l => <Figure>{money(l.price)}</Figure>},
            {header: 'Discount', cell: l => <Figure muted>{l.discount ? money(l.discount) : '—'}</Figure>},
            {header: 'Status', cell: l => <StateToken state={l.order_item_status} />},
          ]}
        />
      </PageSection>

      <PageSection title="Totals">
        <MetadataList columns="multi">
          <TargetFact label="Subtotal" target={{kind: 'total', label: `Subtotal ${money(t.subtotal)}`, ids: {total: 'subtotal', ...ids}}}>
            <Text hasTabularNumbers>{money(t.subtotal)}</Text>
          </TargetFact>
          {t.discounts.map((d, i) => {
            const amount = d.amount ?? (isMoney(d) ? d : null);
            return (
              <TargetFact key={i} label={d.label ?? 'Discount'} target={{kind: 'total', label: `${d.label ?? 'Discount'} ${money(amount)}`, ids: {total: `discount_${i}`, ...ids}}}>
                <Text hasTabularNumbers>{money(amount)}</Text>
              </TargetFact>
            );
          })}
          <TargetFact label="Shipping" target={{kind: 'total', label: `Shipping ${money(t.shipping)}`, ids: {total: 'shipping', ...ids}}}>
            <Text hasTabularNumbers>{money(t.shipping)}</Text>
          </TargetFact>
          <TargetFact label="Total" target={{kind: 'total', label: `Total ${money(t.total)}`, ids: {total: 'total', ...ids}}}>
            <Text weight="semibold" hasTabularNumbers>
              {money(t.total)}
            </Text>
          </TargetFact>
        </MetadataList>
        {o.coupons_recorded.length > 0 ? (
          <Text type="supporting" color="secondary">
            {`Coupons recorded: ${o.coupons_recorded.map(c => `${c.code} (${c.promotion_id})`).join(', ')}`}
          </Text>
        ) : null}
      </PageSection>

      <PageSection title="Delivery">
        {deliveryError ? <ErrorState error={deliveryError} title="The delivery could not be loaded." /> : null}
        {delivery || o.delivery ? (
          <MetadataList columns="multi">
            <MetadataListItem label="State">
              <StateToken state={delivery?.delivery_state ?? o.delivery?.delivery_state} />
            </MetadataListItem>
            <MetadataListItem label="To">{address(delivery?.delivery_address ?? o.delivery?.delivery_address)}</MetadataListItem>
            <MetadataListItem label="Expected by">{dateTime(delivery?.expected_arrival_until ?? o.delivery?.expected_arrival_until)}</MetadataListItem>
            {o.delivery?.delivered_at ? <MetadataListItem label="Delivered">{dateTime(o.delivery.delivered_at)}</MetadataListItem> : null}
            {delivery?.carrier ? <MetadataListItem label="Carrier">{`${delivery.carrier}${delivery.tracking_number ? ` · ${delivery.tracking_number}` : ''}`}</MetadataListItem> : null}
          </MetadataList>
        ) : (
          <Text color="secondary">No delivery yet.</Text>
        )}
        {trackingError ? (
          trackingError.code === 'tracking_not_found' ? (
            <Text color="secondary">The carrier has not reported anything yet.</Text>
          ) : (
            <ErrorState error={trackingError} title="Tracking could not be loaded." />
          )
        ) : null}
        {tracking ? (
          <DataTable
            rows={tracking}
            rowKey={e => `${e.at}-${e.delivery_state}`}
            empty={{title: 'No tracking events yet'}}
            target={e => ({
              kind: 'tracking_event',
              label: `${dateTime(e.at)} ${e.delivery_state}${e.location ? ` at ${e.location}` : ''}`,
              ids: {tracking_event: `${e.at}:${e.delivery_state}`, delivery_id: delivery?.id ?? o.delivery?.id ?? ''},
            })}
            columns={[
              {header: 'When', cell: e => <Text>{dateTime(e.at)}</Text>},
              {header: 'State', cell: e => <StateToken state={e.delivery_state} />},
              {header: 'Where', cell: e => <Text color="secondary">{e.location ?? '—'}</Text>},
            ]}
          />
        ) : null}
      </PageSection>
    </PageFrame>
  );
}

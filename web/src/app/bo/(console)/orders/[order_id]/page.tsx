import {ErrorState} from '@/components/bo/ErrorState';
import {OrderView} from '@/components/bo/OrderView';
import {PageFrame} from '@/components/bo/PageFrame';

import {read, type Result} from '../../../_lib/server';
import type {Delivery, Order, Page, Payment, TrackingEvent} from '../../../_lib/types';

export const metadata = {title: 'Order'};

// One order: get_order, then get_payment, get_delivery and track_shipment together.
export default async function OrderPage({params}: {params: Promise<{order_id: string}>}) {
  const {order_id: id} = await params;
  const enc = encodeURIComponent(id);
  const order = await read<Order>(`/v1/sales/orders/${enc}`);
  if (!order.ok) {
    return (
      <PageFrame back={{href: '/bo/orders', label: 'Orders'}} title={id}>
        <ErrorState error={order.error} title="This order could not be loaded." />
      </PageFrame>
    );
  }
  const deliveryId = order.data.delivery?.id;
  const none = Promise.resolve(null);
  const [payment, delivery, tracking] = await Promise.all([
    read<Payment>(`/v1/sales/orders/${enc}/payment`),
    deliveryId ? read<Delivery>(`/v1/fulfillment/deliveries/${encodeURIComponent(deliveryId)}`) : none,
    deliveryId
      ? read<Page<TrackingEvent>>(`/v1/fulfillment/deliveries/${encodeURIComponent(deliveryId)}/tracking`, {mode: 'delivery', delivery_id: deliveryId, limit: 50})
      : none,
  ]);
  const ok = <T,>(r: Result<T> | null) => (r && r.ok ? r.data : null);
  const err = <T,>(r: Result<T> | null) => (r && !r.ok ? r.error : null);

  return (
    <OrderView
      order={order.data}
      payment={ok(payment)}
      paymentError={err(payment)}
      delivery={ok(delivery)}
      deliveryError={err(delivery)}
      tracking={ok(tracking)?.items ?? null}
      trackingError={err(tracking)}
    />
  );
}

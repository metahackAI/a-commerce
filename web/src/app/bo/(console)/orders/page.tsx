import {ErrorState} from '@/components/bo/ErrorState';
import {FilterBar} from '@/components/bo/FilterBar';
import {PageFrame} from '@/components/bo/PageFrame';
import {Pager} from '@/components/bo/Pager';
import {OrderList} from '@/components/bo/lists/sales';

import {param, params, read, type SearchParams} from '../../_lib/server';
import type {OrderSummary, Page} from '../../_lib/types';

export const metadata = {title: 'Orders'};

const STATUSES = ['payment_due', 'processing', 'in_transit', 'delivered', 'returned', 'cancelled', 'problem'];
const label = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, ' ');

// Orders (search_orders): by order number, customer (email or name, as people say it), status
// and date.
export default async function OrdersPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const values = {
    order_number: param(sp, 'order_number'),
    customer: param(sp, 'customer') ?? param(sp, 'customer_id'),
    order_status: params(sp, 'order_status'),
    order_date_from: param(sp, 'order_date_from'),
    order_date_to: param(sp, 'order_date_to'),
    sort: param(sp, 'sort'),
  };
  const cursor = param(sp, 'cursor');
  const list = await read<Page<OrderSummary>>('/v1/sales/orders', {...values, limit: 50, cursor});

  return (
    <PageFrame
      title="Orders"
      description="Every order in the store. Open one for its lines, payment, delivery and tracking."
      toolbar={
        <FilterBar
          values={values}
          filters={[
            {kind: 'text', key: 'order_number', label: 'Order number', placeholder: 'e.g. 20402', width: 140},
            {kind: 'text', key: 'customer', label: 'Customer', placeholder: 'Email or name', width: 200},
            {kind: 'multi', key: 'order_status', label: 'Status', options: STATUSES.map(s => ({value: s, label: s === 'problem' ? 'Needs attention' : label(s)}))},
            {kind: 'date', key: 'order_date_from', label: 'Placed from'},
            {kind: 'date', key: 'order_date_to', label: 'Placed through'},
            {
              kind: 'select',
              key: 'sort',
              label: 'Sort',
              placeholder: 'Newest first',
              options: [
                {value: '-order_date', label: 'Newest first'},
                {value: 'order_date', label: 'Oldest first'},
                {value: '-total', label: 'Largest total first'},
                {value: 'total', label: 'Smallest total first'},
              ],
            },
          ]}
        />
      }>
      {list.ok ? (
        <>
          <OrderList items={list.data.items} />
          <Pager base="/bo/orders" params={{...values, cursor}} shown={list.data.items.length} total={list.data.total} nextCursor={list.data.next_cursor} noun="orders" />
        </>
      ) : (
        <ErrorState error={list.error} title="The orders could not be loaded." />
      )}
    </PageFrame>
  );
}

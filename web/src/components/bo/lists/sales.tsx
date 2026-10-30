'use client';

// Lists of the sales and fulfillment domains: orders and return requests.

import {VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';

import {count, date, dateTime, money, words} from '@/app/bo/_lib/format';
import type {OrderSummary, ReturnRequest} from '@/app/bo/_lib/types';

import {StateToken} from '../StateToken';
import {DataTable, Figure, RecordCell} from '../tables';

export function OrderList({items}: {items: OrderSummary[]}) {
  return (
    <DataTable
      rows={items}
      rowKey={r => r.id}
      empty={{title: 'No orders match', description: 'Try other filters.'}}
      target={r => ({
        kind: 'order_row',
        label: `Order #${r.order_number} · ${r.order_status} · ${money(r.total)} · ${date(r.order_date)}`,
        ids: {order_id: r.id},
        term: 'schema:Order',
      })}
      columns={[
        {header: 'Order', cell: r => <RecordCell href={`/bo/orders/${encodeURIComponent(r.id)}`} name={`#${r.order_number}`} id={r.id} />},
        {header: 'Status', cell: r => <StateToken state={r.order_status} />},
        {header: 'Customer', cell: r => <Link href={`/bo/customers/${encodeURIComponent(r.customer_id)}`}>{r.customer_id}</Link>},
        {header: 'Placed', cell: r => <Text>{dateTime(r.order_date)}</Text>},
        {header: 'Total', cell: r => <Figure>{money(r.total)}</Figure>},
      ]}
    />
  );
}

export function ReturnRequestList({items}: {items: ReturnRequest[]}) {
  return (
    <DataTable
      rows={items}
      rowKey={r => r.claim_id}
      empty={{title: 'No return requests match', description: 'Nothing is waiting with these filters.'}}
      target={r => ({
        kind: 'request_row',
        label: `Return #${r.order_number} ${r.item} ×${r.quantity} · ${money(r.amount)} · ${r.reason} · ${r.photos} photos · day ${r.days_since_delivery ?? '?'}`,
        ids: {claim_id: r.claim_id},
        term: 'acom:Claim',
      })}
      columns={[
        {header: 'Request', cell: r => <RecordCell href={`/bo/claims/${encodeURIComponent(r.claim_id)}`} name={`#${r.order_number} · ${r.item}`} id={r.claim_id} />},
        {header: 'Qty', cell: r => <Figure>{count(r.quantity)}</Figure>},
        {header: 'Amount', cell: r => <Figure>{money(r.amount)}</Figure>},
        {
          header: 'Reason',
          cell: r => (
            <VStack gap={0}>
              <Text>{words(r.reason)}</Text>
              {r.reason_text ? (
                <Text type="supporting" color="secondary" maxLines={2}>
                  {`“${r.reason_text}”`}
                </Text>
              ) : null}
            </VStack>
          ),
        },
        {header: 'Photos', cell: r => <Figure muted={r.photos === 0}>{count(r.photos)}</Figure>},
        {
          header: 'Since delivery',
          cell: r => (
            <VStack gap={0}>
              <Text>{r.days_since_delivery !== null && r.days_since_delivery !== undefined ? `${r.days_since_delivery} days` : '—'}</Text>
              <Text type="supporting" color="secondary">
                {date(r.delivered_at)}
              </Text>
            </VStack>
          ),
        },
        {header: 'Requested', cell: r => <Text>{dateTime(r.requested_at)}</Text>},
        {header: 'Status', cell: r => <StateToken state={r.status} />},
      ]}
    />
  );
}

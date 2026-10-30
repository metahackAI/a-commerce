import {ErrorState} from '@/components/bo/ErrorState';
import {FilterBar} from '@/components/bo/FilterBar';
import {PageFrame} from '@/components/bo/PageFrame';
import {Pager} from '@/components/bo/Pager';
import {ReturnRequestList} from '@/components/bo/lists/sales';

import {param, params, read, readOptional, type SearchParams} from '../../_lib/server';
import type {Page, ReturnRequest} from '../../_lib/types';

export const metadata = {title: 'Returns'};

const STATUSES = [
  'charge_due',
  'requested',
  'photos_requested',
  'approved',
  'pickup_instructed',
  'picked_up',
  'inspected',
  'reship_requested',
  'completed',
  'declined',
  'withdrawn',
  'converted',
];
const REASONS = ['defect', 'damaged_in_transit', 'not_as_described', 'wrong_item_sent', 'wrong_option_sent', 'missing_parts', 'wrong_size', 'different_option_wanted', 'changed_mind'];
const label = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, ' ');

// Return requests (search_return_requests), oldest waiting first. Open one to see the policy's
// suggestion (evaluate_return_request) and refund it.
export default async function ReturnsPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const values = {
    status: params(sp, 'status'),
    reason: params(sp, 'reason'),
    min_amount: param(sp, 'min_amount'),
    sort: param(sp, 'sort'),
  };
  const cursor = param(sp, 'cursor');

  // The minimum amount is a MonetaryAmount in the store's currency; the store's currency is read
  // from the requests themselves (the web does not assume one).
  let currency = param(sp, 'currency');
  if (values.min_amount && !currency) {
    const probe = await readOptional<Page<ReturnRequest>>('/v1/fulfillment/return-requests', {status: values.status, limit: 1});
    currency = probe?.items[0]?.amount.currency;
  }
  const min = values.min_amount && Number.isFinite(Number(values.min_amount)) ? values.min_amount : undefined;

  const list = await read<Page<ReturnRequest>>('/v1/fulfillment/return-requests', {
    status: values.status,
    reason: values.reason,
    'min_amount.value': min && currency ? min : undefined,
    'min_amount.currency': min && currency ? currency : undefined,
    sort: values.sort,
    limit: 50,
    cursor,
  });

  return (
    <PageFrame
      title="Returns"
      description="Return requests waiting for a decision, oldest first. Refunds over the policy’s limit need a person’s approval in a change set."
      toolbar={
        <FilterBar
          values={values}
          filters={[
            {kind: 'multi', key: 'status', label: 'Status', placeholder: 'Requested', options: STATUSES.map(s => ({value: s, label: label(s)}))},
            {kind: 'multi', key: 'reason', label: 'Reason', options: REASONS.map(s => ({value: s, label: label(s)}))},
            {kind: 'text', key: 'min_amount', label: 'At least', placeholder: 'e.g. 200.00', width: 140},
            {
              kind: 'select',
              key: 'sort',
              label: 'Sort',
              placeholder: 'Waiting longest',
              options: [
                {value: 'requested_at', label: 'Waiting longest'},
                {value: '-amount', label: 'Largest amount'},
              ],
            },
          ]}
        />
      }>
      {list.ok ? (
        <>
          <ReturnRequestList items={list.data.items} />
          <Pager base="/bo/returns" params={{...values, cursor}} shown={list.data.items.length} total={list.data.total} nextCursor={list.data.next_cursor} noun="requests" />
        </>
      ) : (
        <ErrorState error={list.error} title="The return requests could not be loaded." />
      )}
    </PageFrame>
  );
}

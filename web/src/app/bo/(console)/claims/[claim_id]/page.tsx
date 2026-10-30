import {ClaimView} from '@/components/bo/ClaimView';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';

import {read, readOptional} from '../../../_lib/server';
import type {Claim, Order, Page, Refund, ReturnEvaluation, ReturnHistory} from '../../../_lib/types';

export const metadata = {title: 'Return'};

// One claim: get_claim, its order (get_order) for names and the customer, the policy's
// suggestion (evaluate_return_request, returns only), the refund (get_refund) and the customer's
// return history (get_return_history).
export default async function ClaimPage({params}: {params: Promise<{claim_id: string}>}) {
  const {claim_id: id} = await params;
  const enc = encodeURIComponent(id);
  const claim = await read<Claim>(`/v1/fulfillment/claims/${enc}`);
  if (!claim.ok) {
    return (
      <PageFrame back={{href: '/bo/returns', label: 'Returns'}} title={id}>
        <ErrorState error={claim.error} title="This claim could not be loaded." />
      </PageFrame>
    );
  }
  const c = claim.data;
  const [order, evaluation, refund] = await Promise.all([
    readOptional<Order>(`/v1/sales/orders/${encodeURIComponent(c.order_id)}`),
    c.claim_type === 'return' ? read<Page<ReturnEvaluation>>(`/v1/fulfillment/return-requests/${enc}/evaluation`) : Promise.resolve(null),
    c.refund_id ? read<Refund>(`/v1/fulfillment/refunds/${encodeURIComponent(c.refund_id)}`) : Promise.resolve(null),
  ]);
  const history = order ? await read<ReturnHistory>(`/v1/customer/people/${encodeURIComponent(order.customer_id)}/returns`, {limit: 20}) : null;

  return (
    <ClaimView
      claim={c}
      order={order}
      evaluation={evaluation?.ok ? (evaluation.data.items[0] ?? null) : null}
      evaluationError={evaluation && !evaluation.ok ? evaluation.error : null}
      refund={refund?.ok ? refund.data : null}
      refundError={refund && !refund.ok ? refund.error : null}
      history={history?.ok ? history.data : null}
      historyError={history && !history.ok ? history.error : null}
    />
  );
}

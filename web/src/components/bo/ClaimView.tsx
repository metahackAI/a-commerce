'use client';

// One claim (get_claim): the request, the policy's suggestion (evaluate_return_request), the
// refund if one exists (get_refund), and the customer's recent returns (get_return_history).
// Refunding is a write (refund_return): preview shows the amount the server computed, the checks
// and whether a person must approve it (over the refund threshold); otherwise it can be applied.

import {useRouter} from 'next/navigation';
import {VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {Banner} from '@astryxdesign/core/Banner';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';

import type {ApiErrorBody} from '@/lib/api';
import {count, date, dateTime, money, words} from '@/app/bo/_lib/format';
import {withParams} from '@/app/bo/_lib/links';
import type {Claim, Order, Refund, ReturnEvaluation, ReturnHistory} from '@/app/bo/_lib/types';

import {useBoSession} from './BoSession';
import {ErrorState} from './ErrorState';
import {PageFrame, PageSection} from './PageFrame';
import {StateToken} from './StateToken';
import {DataTable, Figure} from './tables';
import {TargetCard, TargetFact} from './targets';
import {WriteFlow} from './WriteFlow';

export interface ClaimViewProps {
  claim: Claim;
  order: Order | null;
  evaluation: ReturnEvaluation | null;
  evaluationError: ApiErrorBody | null;
  refund: Refund | null;
  refundError: ApiErrorBody | null;
  history: ReturnHistory | null;
  historyError: ApiErrorBody | null;
}

export function ClaimView({claim: c, order, evaluation: e, evaluationError, refund, refundError, history, historyError}: ClaimViewProps) {
  const router = useRouter();
  const session = useBoSession();
  const canRefund =
    c.claim_type === 'return' && c.status === 'requested' && !c.refund_id && (session?.scopes.includes('tools:write:fulfillment') ?? false);
  const ids = {claim_id: c.id};
  const lines = new Map((order?.ordered_item ?? []).map(l => [l.id, l]));

  return (
    <PageFrame
      back={{href: '/bo/returns', label: 'Returns'}}
      title={`${words(c.claim_type)} ${order ? `for order #${order.order_number}` : c.id}`}
      meta={
        <>
          <StateToken state={c.status} size="md" />
          <Text type="supporting" color="secondary">
            {`${c.id} · ${words(c.reason)}${c.liability ? ` · ${words(c.liability)}` : ''}`}
          </Text>
        </>
      }
      panel={
        <MetadataList title="Order">
          <MetadataListItem label="Order">
            <Link href={`/bo/orders/${encodeURIComponent(c.order_id)}`}>{order ? `#${order.order_number}` : c.order_id}</Link>
          </MetadataListItem>
          {order ? (
            <MetadataListItem label="Customer">
              <Link href={`/bo/customers/${encodeURIComponent(order.customer_id)}`}>{order.customer_id}</Link>
            </MetadataListItem>
          ) : null}
          {order ? <MetadataListItem label="Placed">{dateTime(order.order_date)}</MetadataListItem> : null}
          {order?.delivery?.delivered_at ? <MetadataListItem label="Delivered">{dateTime(order.delivery.delivered_at)}</MetadataListItem> : null}
          <MetadataListItem label="Evidence">
            {c.evidence && c.evidence.length > 0 ? c.evidence.map(x => `${words(x.evidence_type)} (${x.id})`).join(', ') : 'None'}
          </MetadataListItem>
        </MetadataList>
      }>
      <PageSection title="What is returned">
        <DataTable
          rows={c.items}
          rowKey={i => i.order_item_id}
          empty={{title: 'No items'}}
          target={i => ({
            kind: 'claim_item',
            label: `${lines.get(i.order_item_id)?.name ?? i.order_item_id} ×${i.quantity}`,
            ids: {order_item_id: i.order_item_id, ...ids},
          })}
          columns={[
            {
              header: 'Item',
              cell: i => {
                const l = lines.get(i.order_item_id);
                return (
                  <VStack gap={0}>
                    <Text weight="medium">{l?.name ?? i.order_item_id}</Text>
                    <Text type="supporting" color="secondary">
                      {[l?.size, l?.color, i.order_item_id].filter(Boolean).join(' · ')}
                    </Text>
                  </VStack>
                );
              },
            },
            {header: 'Qty', cell: i => <Figure>{count(i.quantity)}</Figure>},
            {header: 'Paid', cell: i => <Figure>{lines.get(i.order_item_id) ? money(lines.get(i.order_item_id)?.price) : '—'}</Figure>},
          ]}
        />
      </PageSection>

      <PageSection title="What the policy suggests" description="A suggestion from the store’s return rules. It changes nothing.">
        {evaluationError ? (
          evaluationError.code === 'invalid_state' ? (
            <Text color="secondary">This claim was already handled, so there is nothing to suggest.</Text>
          ) : (
            <ErrorState error={evaluationError} title="The suggestion could not be made." />
          )
        ) : null}
        {e ? (
          <TargetCard
            target={{kind: 'suggestion', label: `Suggest ${e.suggested} under ${e.rule}: ${e.rule_text}`, ids: {suggestion: e.claim_id, ...ids}, term: 'schema:AssessAction'}}
            heading={
              <>
                <StateToken state={e.suggested} size="md" label={words(e.suggested)} />
                <Text>{e.rule_text}</Text>
              </>
            }>
            <MetadataList columns="multi">
              <TargetFact label="Refund" target={{kind: 'refund_amount', label: `Refund amount ${money(e.refund_amount)}`, ids}}>
                <Text weight="semibold" hasTabularNumbers>
                  {e.refund_amount ? money(e.refund_amount) : 'No refund'}
                </Text>
              </TargetFact>
              <MetadataListItem label="Rule">
                <Link href={withParams('/bo/policies', {key: e.rule})}>{e.rule}</Link>
              </MetadataListItem>
              <MetadataListItem label="Fault">{words(e.liability)}</MetadataListItem>
              <MetadataListItem label="Return shipping">{words(e.return_fees)}</MetadataListItem>
              <MetadataListItem label="Needs a person">{e.needs_person ? 'Yes, over the refund threshold' : 'No'}</MetadataListItem>
            </MetadataList>
            {e.flags.map(f => (
              <Banner key={f.rule} status="warning" title={f.detail} description={f.rule} />
            ))}
          </TargetCard>
        ) : null}
      </PageSection>

      {canRefund ? (
        <PageSection
          title="Refund"
          description="The server computes the amount from the claim (coupon discounts taken off) and refunds to the original payment. A refund can’t be undone.">
          <WriteFlow
            method="POST"
            path="/v1/fulfillment/refunds"
            input={{claim_id: c.id}}
            previewLabel="Preview the refund"
            applyLabel="Refund now"
            askWhenApprovalNeeded={`Put the refund for return ${c.id}${order ? ` (order #${order.order_number})` : ''} in a change set for approval.`}
            onApplied={() => router.refresh()}
          />
        </PageSection>
      ) : null}

      {c.refund_id ? (
        <PageSection title="Refund">
          {refundError ? <ErrorState error={refundError} title="The refund could not be loaded." /> : null}
          {refund ? (
            <MetadataList columns="multi">
              <TargetFact label="Amount" target={{kind: 'refund', label: `Refund ${refund.id} ${money(refund.amount)} ${refund.refund_status}`, ids: {refund_id: refund.id}}}>
                <Text weight="semibold">{money(refund.amount)}</Text>
              </TargetFact>
              <MetadataListItem label="Status">
                <StateToken state={refund.refund_status} />
              </MetadataListItem>
              <MetadataListItem label="To">{refund.to}</MetadataListItem>
              <MetadataListItem label="Method">{words(refund.refund_method)}</MetadataListItem>
              <MetadataListItem label="Events">
                {(refund.events ?? []).map(ev => `${words(ev.refund_status)} ${dateTime(ev.at)}`).join(' · ') || '—'}
              </MetadataListItem>
            </MetadataList>
          ) : null}
        </PageSection>
      ) : null}

      <PageSection title="Status history">
        <DataTable
          rows={c.status_history ?? []}
          rowKey={h => `${h.at}-${h.status}`}
          empty={{title: 'No history'}}
          target={h => ({kind: 'claim_history', label: `${dateTime(h.at)} ${h.status}`, ids: {at: h.at, ...ids}})}
          columns={[
            {header: 'When', cell: h => <Text>{dateTime(h.at)}</Text>},
            {header: 'Status', cell: h => <StateToken state={h.status} />},
          ]}
        />
      </PageSection>

      <PageSection
        title="This customer’s returns"
        count={history ? `${history.summary.count} in ${history.summary.days} days` : undefined}
        description="From get_return_history; flagged when it reaches the policy’s threshold.">
        {historyError ? <ErrorState error={historyError} title="The return history could not be loaded." /> : null}
        {history?.summary.flagged ? (
          <Banner status="warning" title={`${history.summary.count} returns in ${history.summary.days} days`} description={`The policy flags ${history.summary.threshold} or more.`} />
        ) : null}
        {history ? (
          <DataTable
            rows={history.items}
            rowKey={h => h.id}
            empty={{title: 'No other returns'}}
            target={h => ({kind: 'return_history', label: `#${h.order_number} ${h.item} ${money(h.amount)} ${h.reason} ${h.status}`, ids: {claim_id: h.id}})}
            columns={[
              {
                header: 'Return',
                cell: h => (
                  <VStack gap={0}>
                    <Link href={`/bo/claims/${encodeURIComponent(h.id)}`}>{`#${h.order_number} · ${h.item}`}</Link>
                    <Text type="supporting" color="secondary">
                      {date(h.requested_at)}
                    </Text>
                  </VStack>
                ),
              },
              {header: 'Amount', cell: h => <Figure>{money(h.amount)}</Figure>},
              {header: 'Reason', cell: h => <Text>{words(h.reason)}</Text>},
              {header: 'Status', cell: h => <StateToken state={h.status} />},
            ]}
          />
        ) : null}
      </PageSection>
    </PageFrame>
  );
}

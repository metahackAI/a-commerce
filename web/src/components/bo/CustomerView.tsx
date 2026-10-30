'use client';

// One member (get_member, masked by default), their tier and why (get_member_tier), their recent
// returns (get_return_history) and orders (search_orders).

import {VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {Banner} from '@astryxdesign/core/Banner';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';

import type {ApiErrorBody} from '@/lib/api';
import {date, dateTime, isMoney, money, value, words} from '@/app/bo/_lib/format';
import type {Member, MemberTier, OrderSummary, ReturnHistory} from '@/app/bo/_lib/types';

import {ErrorState} from './ErrorState';
import {PageFrame, PageSection} from './PageFrame';
import {StateToken} from './StateToken';
import {OrderList} from './lists/sales';
import {DataTable, Figure} from './tables';
import {TargetCard, TargetFact} from './targets';

const amount = (v: unknown) => (isMoney(v) ? money(v) : value(v));

export interface CustomerViewProps {
  member: Member;
  tier: MemberTier | null;
  tierError: ApiErrorBody | null;
  tierNames: Record<string, string>;
  returns: ReturnHistory | null;
  returnsError: ApiErrorBody | null;
  orders: OrderSummary[] | null;
  ordersTotal: number;
  ordersError: ApiErrorBody | null;
}

export function CustomerView({member: m, tier, tierError, tierNames, returns, returnsError, orders, ordersTotal, ordersError}: CustomerViewProps) {
  const ids = {customer_id: m.id};
  const tierName = (id: string | null | undefined) => (id ? (tierNames[id] ?? id) : '—');

  return (
    <PageFrame
      back={{href: '/bo/customers', label: 'Customers'}}
      title={m.name}
      meta={
        <>
          <Text type="supporting" color="secondary">
            {m.id}
          </Text>
          <StateToken state={m.state} size="md" />
        </>
      }
      panel={
        <MetadataList title="Member">
          <MetadataListItem label="Type">{words(m.type)}</MetadataListItem>
          <MetadataListItem label="Membership">{m.member_of?.membership_number ?? '—'}</MetadataListItem>
          <MetadataListItem label="Program">{m.member_of?.program_id ?? '—'}</MetadataListItem>
          {m.email ? <MetadataListItem label="Email">{m.email}</MetadataListItem> : null}
          {m['acom:business_verification'] ? <MetadataListItem label="Business">{words(m['acom:business_verification'])}</MetadataListItem> : null}
          <MetadataListItem label="Consent">
            {m.consent && Object.keys(m.consent).length > 0
              ? Object.entries(m.consent)
                  .map(([k, v]) => `${words(k)}: ${v ? 'yes' : 'no'}`)
                  .join(' · ')
              : '—'}
          </MetadataListItem>
        </MetadataList>
      }>
      <PageSection title="Tier">
        {tierError ? (
          tierError.code === 'tier_not_found' ? (
            <Text color="secondary">This member has no tier.</Text>
          ) : (
            <ErrorState error={tierError} title="The tier could not be loaded." />
          )
        ) : null}
        {tier ? (
          <TargetCard
            target={{kind: 'member_tier', label: `${m.id} is ${tier.tier_id} since ${tier.since}: ${tier.why.rule}`, ids: {tier: tier.tier_id, ...ids}, term: 'schema:MemberProgramTier'}}
            heading={
              <>
                <Text weight="semibold">{`${tierName(tier.tier_id)} since ${date(tier.since)}`}</Text>
                <Text type="supporting" color="secondary">
                  {tier.why.rule}
                </Text>
              </>
            }>
            <MetadataList columns="multi">
              <MetadataListItem label="Keeping it">
                <StateToken state={tier.retention.status} />
              </MetadataListItem>
              <MetadataListItem label="Next evaluation">{dateTime(tier.next_evaluation)}</MetadataListItem>
              {tier.why.value !== null && tier.why.value !== undefined ? <MetadataListItem label="Counted so far">{amount(tier.why.value)}</MetadataListItem> : null}
              <MetadataListItem label="Decided by">{words(tier.source)}</MetadataListItem>
              {tier.override ? <MetadataListItem label="Set by hand">{value(tier.override)}</MetadataListItem> : null}
              {tier.has_tier_benefit && tier.has_tier_benefit.length > 0 ? (
                <MetadataListItem label="Benefits">{tier.has_tier_benefit.map(b => words(b.type)).join(', ')}</MetadataListItem>
              ) : null}
            </MetadataList>
            {tier.progress.map(p => (
              <TargetFact
                key={p.to_tier_id}
                label={`To ${tierName(p.to_tier_id)}`}
                target={{kind: 'tier_progress', label: `To ${p.to_tier_id}: ${amount(p.remaining)} to go (${p.metric})`, ids: {to_tier: p.to_tier_id, ...ids}}}>
                <VStack gap={0}>
                  <Text>{`${amount(p.current)} of ${amount(p.threshold)} ${words(p.metric).toLowerCase()} · ${amount(p.remaining)} to go`}</Text>
                  {p.pending ? (
                    <Text type="supporting" color="secondary">
                      {`${money(p.pending)} pending${p.pending.detail ? `: ${p.pending.detail}` : ''}`}
                    </Text>
                  ) : null}
                </VStack>
              </TargetFact>
            ))}
          </TargetCard>
        ) : null}
      </PageSection>

      <PageSection title="Returns" count={returns ? `${returns.summary.count} in ${returns.summary.days} days` : undefined}>
        {returnsError ? <ErrorState error={returnsError} title="The returns could not be loaded." /> : null}
        {returns?.summary.flagged ? (
          <Banner status="warning" title={`${returns.summary.count} returns in ${returns.summary.days} days`} description={`The policy flags ${returns.summary.threshold} or more.`} />
        ) : null}
        {returns ? (
          <DataTable
            rows={returns.items}
            rowKey={h => h.id}
            empty={{title: 'No returns in this period'}}
            target={h => ({kind: 'return_history', label: `#${h.order_number} ${h.item} ${money(h.amount)} ${h.reason} ${h.status}`, ids: {claim_id: h.id}})}
            columns={[
              {
                header: 'Return',
                cell: h => (
                  <VStack gap={0}>
                    <Link href={`/bo/claims/${encodeURIComponent(h.id)}`}>{`#${h.order_number} · ${h.item}`}</Link>
                    <Text type="supporting" color="secondary">
                      {dateTime(h.requested_at)}
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

      <PageSection title="Orders" count={orders ? `${ordersTotal}` : undefined}>
        {ordersError ? <ErrorState error={ordersError} title="The orders could not be loaded." /> : null}
        {orders ? <OrderList items={orders} /> : null}
        {orders && ordersTotal > orders.length ? (
          <Link href={`/bo/orders?customer=${encodeURIComponent(m.id)}`}>{`See all ${ordersTotal} orders`}</Link>
        ) : null}
      </PageSection>
    </PageFrame>
  );
}

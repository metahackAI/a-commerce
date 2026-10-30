import {CustomerView} from '@/components/bo/CustomerView';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';

import {read, readOptional} from '../../../_lib/server';
import type {Member, MemberTier, OrderSummary, Page, ReturnHistory, StoreMessage, Tier} from '../../../_lib/types';

export const metadata = {title: 'Customer'};

// One member: get_member, get_member_tier, get_return_history, their orders (search_orders) and
// the messages the store sent them (list_notifications), with tier names from list_tiers.
export default async function CustomerPage({params}: {params: Promise<{customer_id: string}>}) {
  const {customer_id: id} = await params;
  const enc = encodeURIComponent(id);
  const member = await read<Member>(`/v1/customer/people/${enc}`);
  if (!member.ok) {
    return (
      <PageFrame back={{href: '/bo/customers', label: 'Customers'}} title={id}>
        <ErrorState error={member.error} title="This member could not be loaded." />
      </PageFrame>
    );
  }
  const [tier, returns, orders, tiers, messages] = await Promise.all([
    read<MemberTier>(`/v1/customer/people/${enc}/tier`),
    read<ReturnHistory>(`/v1/customer/people/${enc}/returns`, {limit: 20}),
    read<Page<OrderSummary>>('/v1/sales/orders', {customer_id: id, limit: 10}),
    readOptional<Page<Tier>>('/v1/customer/tiers', {limit: 100}),
    read<Page<StoreMessage>>('/v1/messaging/notifications', {customer_id: id, limit: 20}),
  ]);

  return (
    <CustomerView
      member={member.data}
      tier={tier.ok ? tier.data : null}
      tierError={tier.ok ? null : tier.error}
      tierNames={Object.fromEntries((tiers?.items ?? []).map(t => [t.id, t.name]))}
      returns={returns.ok ? returns.data : null}
      returnsError={returns.ok ? null : returns.error}
      orders={orders.ok ? orders.data.items : null}
      ordersTotal={orders.ok ? orders.data.total : 0}
      ordersError={orders.ok ? null : orders.error}
      messages={messages.ok ? messages.data : null}
      messagesError={messages.ok ? null : messages.error}
    />
  );
}

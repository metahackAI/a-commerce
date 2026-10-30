import {ErrorState} from '@/components/bo/ErrorState';
import {FilterBar} from '@/components/bo/FilterBar';
import {PageFrame} from '@/components/bo/PageFrame';
import {Pager} from '@/components/bo/Pager';
import {MemberList, TierSummary} from '@/components/bo/lists/customer';

import {param, read, readOptional, type SearchParams} from '../../_lib/server';
import type {MemberSummary, Page, Tier} from '../../_lib/types';

export const metadata = {title: 'Customers'};

const label = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, ' ');

// Customers: members (search_members, masked by default) and the store's tiers (list_tiers).
export default async function CustomersPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const values = {
    query: param(sp, 'query'),
    state: param(sp, 'state'),
    tier_id: param(sp, 'tier_id'),
    type: param(sp, 'type'),
    joined_from: param(sp, 'joined_from'),
    joined_through: param(sp, 'joined_through'),
  };
  const cursor = param(sp, 'cursor');
  const [list, tiers] = await Promise.all([
    read<Page<MemberSummary>>('/v1/customer/people', {...values, limit: 50, cursor}),
    readOptional<Page<Tier>>('/v1/customer/tiers', {limit: 100}),
  ]);
  const tierNames = Object.fromEntries((tiers?.items ?? []).map(t => [t.id, t.name]));

  return (
    <PageFrame
      title="Customers"
      description="Members of the store. Names and contacts are masked; open a member for their tier, returns and orders."
      toolbar={
        <FilterBar
          values={values}
          filters={[
            {kind: 'text', key: 'query', label: 'Search', placeholder: 'Id or part of a name'},
            {kind: 'select', key: 'tier_id', label: 'Tier', options: (tiers?.items ?? []).map(t => ({value: t.id, label: t.name}))},
            {kind: 'select', key: 'state', label: 'State', options: ['active', 'dormant', 'blocked', 'pending_withdrawal', 'withdrawn'].map(s => ({value: s, label: label(s)}))},
            {kind: 'select', key: 'type', label: 'Type', options: ['individual', 'business'].map(s => ({value: s, label: label(s)}))},
            {kind: 'date', key: 'joined_from', label: 'Joined from'},
            {kind: 'date', key: 'joined_through', label: 'Joined through'},
          ]}
        />
      }>
      {tiers ? <TierSummary tiers={tiers.items} /> : null}
      {list.ok ? (
        <>
          <MemberList items={list.data.items} tierNames={tierNames} />
          <Pager base="/bo/customers" params={{...values, cursor}} shown={list.data.items.length} total={list.data.total} nextCursor={list.data.next_cursor} noun="members" />
        </>
      ) : (
        <ErrorState error={list.error} title="The members could not be loaded." />
      )}
    </PageFrame>
  );
}

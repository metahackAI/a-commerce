'use client';

// Lists of the customer domain: members and tiers.

import {Text} from '@astryxdesign/core/Text';
import {MetadataList} from '@astryxdesign/core/MetadataList';

import {count} from '@/app/bo/_lib/format';
import {withParams} from '@/app/bo/_lib/links';
import type {MemberSummary, Tier} from '@/app/bo/_lib/types';

import {StateToken} from '../StateToken';
import {DataTable, RecordCell} from '../tables';
import {TargetFact} from '../targets';

export function MemberList({items, tierNames}: {items: MemberSummary[]; tierNames: Record<string, string>}) {
  return (
    <DataTable
      rows={items}
      rowKey={r => r.id}
      empty={{title: 'No members match', description: 'Names and contacts are masked; search by id or part of a name.'}}
      target={r => ({kind: 'member_row', label: `Member ${r.id} (${r.name}) · ${r.tier_id ?? 'no tier'} · ${r.state}`, ids: {customer_id: r.id}, term: 'schema:Person'})}
      columns={[
        {header: 'Member', cell: r => <RecordCell href={`/bo/customers/${encodeURIComponent(r.id)}`} name={r.name} id={r.id} />},
        {header: 'Email', cell: r => <Text color="secondary">{r.email ?? '—'}</Text>},
        {header: 'Tier', cell: r => <Text>{r.tier_id ? (tierNames[r.tier_id] ?? r.tier_id) : '—'}</Text>},
        {header: 'State', cell: r => <StateToken state={r.state} />},
      ]}
    />
  );
}

export function TierSummary({tiers}: {tiers: Tier[]}) {
  if (tiers.length === 0) return null;
  return (
    <MetadataList columns="multi" title="Tiers">
      {tiers.map(t => (
        <TargetFact
          key={t.id}
          label={`${t.name} (${t.id})`}
          target={{kind: 'tier', label: `Tier ${t.name}: ${t.member_count} members`, ids: {tier_id: t.id}, term: 'schema:MemberProgramTier'}}>
          <RecordCell href={withParams('/bo/customers', {tier_id: t.id})} name={`${count(t.member_count)} members`} />
        </TargetFact>
      ))}
    </MetadataList>
  );
}

'use client';

// Lists of the platform domain: change sets, policies.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';

import {count, dateTime, value} from '@/app/bo/_lib/format';
import {withParams} from '@/app/bo/_lib/links';
import type {ChangeSetSummary, PolicyFile, PolicyValue} from '@/app/bo/_lib/types';

import {StateToken} from '../StateToken';
import {DataTable, Figure, RecordCell} from '../tables';

export function ChangeSetList({items}: {items: ChangeSetSummary[]}) {
  return (
    <DataTable
      rows={items}
      rowKey={r => r.id}
      empty={{title: 'No change sets match', description: 'Change sets appear when the agent prepares a write for you to approve.'}}
      target={r => ({
        kind: 'change_set',
        label: `#${r.number} ${r.title} · ${r.state} · v${r.version}`,
        ids: {change_set_id: r.id},
        term: 'acom:ChangeSet',
      })}
      columns={[
        {header: 'Change set', cell: r => <RecordCell href={`/bo/change-sets/${encodeURIComponent(r.id)}`} name={`#${r.number} · ${r.title}`} id={r.id} />},
        {
          header: 'State',
          cell: r => (
            <HStack gap={1} wrap="wrap">
              <StateToken state={r.state} />
              {r.needs_approval ? <Token label="Needs approval" size="sm" color="orange" /> : null}
            </HStack>
          ),
        },
        {header: 'Version', cell: r => <Figure>{`v${r.version}`}</Figure>},
        {header: 'Items', cell: r => <Figure>{count(r.item_count)}</Figure>},
        {
          header: 'Created',
          cell: r => (
            <VStack gap={0}>
              <Text>{dateTime(r.created_at)}</Text>
              <Text type="supporting" color="secondary">
                {r.created_by}
              </Text>
            </VStack>
          ),
        },
        {header: 'Undo until', cell: r => <Text color={r.undo_until ? undefined : 'secondary'}>{r.undo_until ? dateTime(r.undo_until) : '—'}</Text>},
      ]}
    />
  );
}

export function PolicyFileList({items, selected}: {items: PolicyFile[]; selected?: string}) {
  return (
    <DataTable
      rows={items}
      rowKey={r => r.file}
      empty={{title: 'No policy files', description: 'The store has no policy files loaded.'}}
      target={r => ({kind: 'policy_file', label: `${r.file} policies (v${r.version}): ${r.summary}`, ids: {policy_file: r.file}, term: 'acom:Policy'})}
      columns={[
        {
          header: 'File',
          cell: r => <RecordCell href={withParams('/bo/policies', {file: r.file})} name={r.file === selected ? `${r.file} (filtering)` : r.file} />,
        },
        {header: 'What it covers', cell: r => <Text>{r.summary}</Text>},
        {header: 'Domain', cell: r => <Text color="secondary">{r.domain}</Text>},
        {header: 'Version', cell: r => <Figure>{String(r.version)}</Figure>},
        {header: 'Store overrides', cell: r => <Figure muted={r.overridden_keys === 0}>{count(r.overridden_keys)}</Figure>},
      ]}
    />
  );
}

export function PolicyValueList({items}: {items: PolicyValue[]}) {
  return (
    <DataTable
      rows={items}
      rowKey={r => `${r.file}:${r.key}`}
      empty={{title: 'No values under this key'}}
      target={r => ({kind: 'policy_value', label: `${r.key} = ${r.display}`, ids: {policy_key: r.key}, term: 'acom:Policy'})}
      columns={[
        {
          header: 'Key',
          cell: r => (
            <VStack gap={0}>
              <Text weight="medium">{r.key}</Text>
              <Text type="supporting" color="secondary">
                {r.file}
              </Text>
            </VStack>
          ),
        },
        {header: 'Means', cell: r => <Text>{r.display}</Text>},
        {header: 'Stored value', cell: r => <Text color="secondary">{value(r.value)}</Text>},
        {header: 'Source', cell: r => (r.overridden_by_store ? <Token label="Store override" size="sm" color="blue" /> : <Token label="Default" size="sm" />)},
      ]}
    />
  );
}

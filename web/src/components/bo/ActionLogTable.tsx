'use client';

// Action log entries (schema.org Actions) as rows: when, who, which tool, on what, how it ended.
// Used by the activity page, a change set's history, and the `action_log` block.

import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {VStack} from '@astryxdesign/core/Layout';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';

import {actionStatus, actor, dateTime, value, words} from '@/app/bo/_lib/format';
import {entityHref} from '@/app/bo/_lib/links';
import type {ActionEntry} from '@/app/bo/_lib/types';

import {StateToken} from './StateToken';
import {TargetRow} from './targets';

function objectId(a: ActionEntry): string | null {
  const o = a.object;
  if (o && typeof o === 'object' && typeof o.id === 'string') return o.id;
  if (typeof a.object === 'string') return a.object;
  return null;
}

export function ActionLogTable({entries, blockId, empty}: {entries: ActionEntry[]; blockId?: string; empty?: string}) {
  if (entries.length === 0) {
    return <EmptyState isCompact title={empty ?? 'No actions recorded'} description="Every tool call, preview, approval and run is recorded here." />;
  }
  return (
    <Table density="compact" verticalAlign="top">
      <TableHeader>
        <TableRow isHeaderRow>
          <TableHeaderCell>When</TableHeaderCell>
          <TableHeaderCell>Who</TableHeaderCell>
          <TableHeaderCell>Action</TableHeaderCell>
          <TableHeaderCell>On</TableHeaderCell>
          <TableHeaderCell>Status</TableHeaderCell>
          <TableHeaderCell>
            <Text type="supporting" color="secondary">
              Ask
            </Text>
          </TableHeaderCell>
        </TableRow>
      </TableHeader>
      <TableBody>
        {entries.map(a => {
          const on = objectId(a);
          const onHref = entityHref(on);
          return (
            <TargetRow
              key={a.id}
              target={{
                kind: 'action_row',
                label: `${a['@type'] ?? 'Action'} · ${a.instrument ?? ''} · ${a.action_status}${on ? ` · ${on}` : ''}`,
                ids: {action_id: a.id},
                term: `schema:${a['@type'] ?? 'Action'}`,
                blockId,
              }}>
              <TableCell>
                <VStack gap={0}>
                  <Link href={`/bo/activity/${encodeURIComponent(a.id)}`}>{dateTime(a.start_time ?? a.end_time)}</Link>
                  <Text type="supporting" color="secondary">
                    {a.id}
                  </Text>
                </VStack>
              </TableCell>
              <TableCell>
                <Text>{actor(a.agent)}</Text>
              </TableCell>
              <TableCell>
                <VStack gap={0}>
                  <Text>{a.instrument ?? '—'}</Text>
                  <Text type="supporting" color="secondary">
                    {words(String(a['@type'] ?? 'Action').replace(/Action$/, ''))}
                  </Text>
                </VStack>
              </TableCell>
              <TableCell>
                <VStack gap={0}>
                  {on ? onHref ? <Link href={onHref}>{on}</Link> : <Text>{on}</Text> : <Text color="secondary">—</Text>}
                  {a.result && typeof a.result === 'object' ? (
                    <Text type="supporting" color="secondary" maxLines={2}>
                      {value(a.result)}
                    </Text>
                  ) : null}
                </VStack>
              </TableCell>
              <TableCell>
                <StateToken state={a.action_status} label={actionStatus(a.action_status)} />
              </TableCell>
            </TargetRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

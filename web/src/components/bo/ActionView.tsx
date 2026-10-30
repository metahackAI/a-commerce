'use client';

// One action from the log (get_action_log): who did what with which tool, on what, the result
// or error, the change set and preview it belongs to, the policy version and who approved it.
// When the entry carries a preview's changes and checks they are shown as tables.

import {VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {CodeBlock} from '@astryxdesign/core/CodeBlock';

import {actor, count, dateTime, value} from '@/app/bo/_lib/format';
import {entityHref, withParams} from '@/app/bo/_lib/links';
import type {ActionEntry, Change, Check} from '@/app/bo/_lib/types';

import {ChangesTable, ChecksList} from './ChangeParts';
import {PageFrame, PageSection} from './PageFrame';
import {StateToken} from './StateToken';
import {TargetFact} from './targets';

const SHOWN = new Set([
  '@type',
  'id',
  'action_status',
  'agent',
  'instrument',
  'object',
  'result',
  'error',
  'start_time',
  'end_time',
  'acom:change_set_id',
  'acom:preview_id',
  'acom:request_id',
  'acom:tokens',
  'acom:latency_ms',
  'acom:policy_version',
  'acom:approved_by',
  'changes',
  'checks',
  'acom:changes',
  'acom:checks',
]);

function asList<T>(v: unknown): T[] | null {
  return Array.isArray(v) ? (v as T[]) : null;
}

export function ActionView({action: a}: {action: ActionEntry}) {
  const ids = {action_id: a.id};
  const changes = asList<Change>(a.changes ?? a['acom:changes']);
  const checks = asList<Check>(a.checks ?? a['acom:checks']);
  const csId = a['acom:change_set_id'];
  const objectId = a.object && typeof a.object === 'object' ? a.object.id : undefined;
  const others = Object.entries(a).filter(([k]) => !SHOWN.has(k));

  return (
    <PageFrame
      back={{href: '/bo/activity', label: 'Activity'}}
      title={`${a['@type'] ?? 'Action'} · ${a.instrument ?? a.id}`}
      meta={
        <>
          <StateToken state={a.action_status} size="md" />
          <Text type="supporting" color="secondary">
            {`${a.id} · ${dateTime(a.start_time)}`}
          </Text>
        </>
      }
      panel={
        <MetadataList title="Context">
          <MetadataListItem label="Change set">{csId ? <Link href={`/bo/change-sets/${encodeURIComponent(csId)}`}>{csId}</Link> : '—'}</MetadataListItem>
          <MetadataListItem label="Preview">{a['acom:preview_id'] ?? '—'}</MetadataListItem>
          <MetadataListItem label="Request">{a['acom:request_id'] ?? '—'}</MetadataListItem>
          <MetadataListItem label="Policy version">{a['acom:policy_version'] ?? '—'}</MetadataListItem>
          <MetadataListItem label="Approved by">{a['acom:approved_by'] ?? '—'}</MetadataListItem>
          <MetadataListItem label="Tokens">
            {a['acom:tokens'] ? `${count(a['acom:tokens'].input)} in · ${count(a['acom:tokens'].output)} out` : '—'}
          </MetadataListItem>
          <MetadataListItem label="Latency">{typeof a['acom:latency_ms'] === 'number' ? `${count(a['acom:latency_ms'])} ms` : '—'}</MetadataListItem>
        </MetadataList>
      }>
      <PageSection title="What happened">
        <MetadataList columns="multi">
          <TargetFact label="Who" target={{kind: 'action_agent', label: `Done by ${actor(a.agent)}`, ids}}>
            <Text weight="medium">{actor(a.agent)}</Text>
          </TargetFact>
          <TargetFact label="Tool" target={{kind: 'action_tool', label: `Tool ${a.instrument ?? '—'}`, ids}}>
            <Link href={withParams('/bo/activity', {instrument: a.instrument ?? undefined})}>{a.instrument ?? '—'}</Link>
          </TargetFact>
          <TargetFact label="On" target={{kind: 'action_object', label: `On ${value(a.object)}`, ids}}>
            {objectId && entityHref(objectId) ? <Link href={entityHref(objectId) ?? '#'}>{value(a.object)}</Link> : <Text>{value(a.object)}</Text>}
          </TargetFact>
          <TargetFact label="When" target={{kind: 'action_time', label: `From ${dateTime(a.start_time)} to ${dateTime(a.end_time)}`, ids}}>
            <Text>{`${dateTime(a.start_time)} → ${dateTime(a.end_time)}`}</Text>
          </TargetFact>
        </MetadataList>
      </PageSection>

      {a.result ? (
        <PageSection title="Result">
          <TargetFact label="Result" target={{kind: 'action_result', label: `Result ${value(a.result)}`, ids}}>
            <Text>{value(a.result)}</Text>
          </TargetFact>
        </PageSection>
      ) : null}

      {a.error ? (
        <PageSection title="Error">
          <Text>{value(a.error)}</Text>
        </PageSection>
      ) : null}

      {changes ? (
        <PageSection title="Changes">
          <ChangesTable changes={changes} ids={ids} />
        </PageSection>
      ) : null}
      {checks ? (
        <PageSection title="Checks">
          <ChecksList checks={checks} ids={ids} />
        </PageSection>
      ) : null}

      {others.length > 0 ? (
        <PageSection title="Everything else recorded">
          <VStack gap={2}>
            <CodeBlock language="json" code={JSON.stringify(Object.fromEntries(others), null, 2)} />
          </VStack>
        </PageSection>
      ) : null}
    </PageFrame>
  );
}

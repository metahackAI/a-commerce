'use client';

// One task (get_task): why it exists (the watcher's threshold and what it saw), what the agent
// was asked, its history, and what it is linked to. The agent can pick up the request from here;
// dismissing is a person's decision (PersonButton).

import {useRouter} from 'next/navigation';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {Button} from '@astryxdesign/core/Button';
import {Blockquote} from '@astryxdesign/core/Blockquote';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';

import {useAi} from '@/ai/AiProvider';
import {PersonButton} from '@/components/PersonButton';
import {dateTime, value, words} from '@/app/bo/_lib/format';
import {entityHref, withParams} from '@/app/bo/_lib/links';
import type {ChangeSet, Task} from '@/app/bo/_lib/types';

import {useAgentDock} from './AgentDock';
import {useBoSession} from './BoSession';
import {PageFrame, PageSection} from './PageFrame';
import {StateToken} from './StateToken';
import {DataTable} from './tables';
import {TargetBlock, TargetFact} from './targets';

const ORIGIN: Record<string, string> = {you_asked: 'You asked', agent_noticed: 'Agent noticed', automation: 'Automation'};

export function TaskView({task, changeSet}: {task: Task; changeSet: ChangeSet | null}) {
  const router = useRouter();
  const ai = useAi();
  const dock = useAgentDock();
  const session = useBoSession();
  const open = task.state === 'needs_you' || task.state === 'working';
  const canDismiss = open && (session?.scopes.includes('human:task') ?? false);
  const ids = {task_id: task.id};

  return (
    <PageFrame
      back={{href: '/bo', label: 'Inbox'}}
      title={task.title}
      meta={
        <>
          <StateToken state={task.state} size="md" />
          <Text type="supporting" color="secondary">
            {`${ORIGIN[task.origin] ?? words(task.origin)} · ${dateTime(task.created_at)}${task.tag ? ` · ${words(task.tag)}` : ''}`}
          </Text>
        </>
      }
      description={task.summary ?? undefined}
      actions={
        <>
          {task.request ? (
            <Button
              label="Ask the agent to work on it"
              onClick={() => {
                dock.open();
                void ai.send(task.request ?? task.title);
              }}
            />
          ) : null}
          {task.related?.change_set_id ? (
            <Button label="Review the change set" variant="primary" href={`/bo/change-sets/${encodeURIComponent(task.related.change_set_id)}`} />
          ) : null}
          {canDismiss ? (
            <PersonButton
              action={{
                label: 'Dismiss',
                method: 'POST',
                path: `/v1/platform/tasks/${encodeURIComponent(task.id)}/dismiss`,
                confirm: `Dismiss “${task.title}”? A linked change set that is still waiting is rejected too.`,
              }}
              variant="secondary"
              onDone={() => router.refresh()}
            />
          ) : null}
        </>
      }
      panel={
        <MetadataList title="Linked to">
          <MetadataListItem label="Change set">
            {task.related?.change_set_id ? (
              <VStack gap={0.5}>
                <Link href={`/bo/change-sets/${encodeURIComponent(task.related.change_set_id)}`}>
                  {changeSet ? `#${changeSet.number} · ${changeSet.title}` : task.related.change_set_id}
                </Link>
                {changeSet ? <StateToken state={changeSet.state} /> : null}
              </VStack>
            ) : (
              'None yet'
            )}
          </MetadataListItem>
          <MetadataListItem label="Conversation">{task.related?.conversation_id ?? '—'}</MetadataListItem>
          <MetadataListItem label="Records">
            {task.related?.entities?.length ? (
              <VStack gap={0.5}>
                {task.related.entities.map(e => {
                  const href = entityHref(e.id);
                  return href ? (
                    <Link key={e.id} href={href}>{`${words(e.entity)} ${e.id}`}</Link>
                  ) : (
                    <Text key={e.id}>{`${words(e.entity)} ${e.id}`}</Text>
                  );
                })}
              </VStack>
            ) : (
              '—'
            )}
          </MetadataListItem>
          <MetadataListItem label="Updated">{dateTime(task.updated_at)}</MetadataListItem>
        </MetadataList>
      }>
      {task.request ? (
        <PageSection title="What the agent was asked">
          <TargetBlock target={{kind: 'task_request', label: task.request, ids, term: 'acom:Task'}}>
            <Blockquote>{task.request}</Blockquote>
          </TargetBlock>
        </PageSection>
      ) : null}

      {task.watcher ? (
        <PageSection title="Why it appeared" description="A watcher checks the store on a schedule against a policy threshold.">
          <MetadataList columns="multi">
            <TargetFact label="Watcher" target={{kind: 'watcher', label: `Watcher ${task.watcher.key}`, ids: {watcher: task.watcher.key, ...ids}}}>
              <Text weight="medium">{words(task.watcher.key)}</Text>
            </TargetFact>
            {task.watcher.threshold ? (
              <TargetFact
                label="Threshold"
                target={{
                  kind: 'policy_value',
                  label: `${task.watcher.threshold.policy_key} = ${value(task.watcher.threshold.value)}`,
                  ids: {policy_key: task.watcher.threshold.policy_key, ...ids},
                }}>
                <VStack gap={0}>
                  <Text weight="medium">{value(task.watcher.threshold.value)}</Text>
                  <Link href={withParams('/bo/policies', {key: task.watcher.threshold.policy_key})} color="secondary">
                    {task.watcher.threshold.policy_key}
                  </Link>
                </VStack>
              </TargetFact>
            ) : null}
            {Object.entries(task.watcher.observed ?? {}).map(([k, v]) => (
              <TargetFact key={k} label={words(k)} target={{kind: 'observed', label: `${words(k)}: ${value(v)}`, ids: {observed: k, ...ids}}}>
                <Text weight="medium">{k.endsWith('_at') && typeof v === 'string' ? dateTime(v) : value(v)}</Text>
              </TargetFact>
            ))}
          </MetadataList>
        </PageSection>
      ) : null}

      <PageSection title="History">
        <DataTable
          rows={task.history}
          rowKey={h => `${h.at}-${h.state}`}
          empty={{title: 'No history yet'}}
          target={h => ({kind: 'task_history', label: `${dateTime(h.at)} ${h.state}${h.note ? `: ${h.note}` : ''}`, ids: {at: h.at, ...ids}})}
          columns={[
            {header: 'When', cell: h => <Text>{dateTime(h.at)}</Text>},
            {header: 'State', cell: h => <StateToken state={h.state} />},
            {header: 'Note', cell: h => <Text>{h.note ?? '—'}</Text>},
          ]}
        />
      </PageSection>

      {!task.request && !task.watcher ? (
        <HStack gap={2}>
          <Text color="secondary">This task has no request text.</Text>
        </HStack>
      ) : null}
    </PageFrame>
  );
}

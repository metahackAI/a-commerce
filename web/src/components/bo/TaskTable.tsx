'use client';

// The operator's tasks (list_tasks) as the inbox shows them: what needs a decision as cards, and
// what the agent is working on as a quiet column beside them. Finished work is on the change sets
// and activity pages, not here. Every card and item is an AI target; Dismiss is a person's decision
// and goes through PersonButton.

import {useRouter} from 'next/navigation';
import {Card} from '@astryxdesign/core/Card';
import {Divider} from '@astryxdesign/core/Divider';
import {HStack, StackItem, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {Button} from '@astryxdesign/core/Button';
import {ProgressBar} from '@astryxdesign/core/ProgressBar';
import {StatusDot} from '@astryxdesign/core/StatusDot';
import {Token} from '@astryxdesign/core/Token';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {Fragment} from 'react';

import {PersonButton} from '@/components/PersonButton';
import {useAiTarget, type AiTarget} from '@/ai/useAiTarget';
import {clockFace, dateTime, words} from '@/app/bo/_lib/format';
import type {ChangeSet, TaskState, TaskSummary} from '@/app/bo/_lib/types';

import {useAgentDock} from './AgentDock';
import {useBoSession} from './BoSession';
import {StateToken} from './StateToken';
import type {IntentsFor as IntentsLookup} from './ChangeParts';
import {TargetBlock} from './targets';

export const TASK_GROUPS: {state: TaskState; title: string; hint?: string; empty: string}[] = [
  {state: 'needs_you', title: 'Needs your decision', hint: 'Nothing changes in the store until you approve.', empty: 'Nothing is waiting for you.'},
  {state: 'working', title: 'Agent is working', hint: 'It comes back here when it needs you.', empty: 'The agent is not working on anything right now.'},
  {state: 'done', title: 'Done', hint: 'What went live, and what can still be undone.', empty: 'Nothing finished yet.'},
  {state: 'dismissed', title: 'Dismissed', empty: 'Nothing dismissed.'},
];

export const ORIGIN: Record<string, string> = {you_asked: 'You asked', agent_noticed: 'Agent noticed', automation: 'Automation'};

export function dot(state: string) {
  switch (state) {
    case 'needs_you':
      return <StatusDot variant="warning" label="Needs you" />;
    case 'working':
      // Neutral, not the brand's red accent: working is activity, not an alarm.
      return <StatusDot variant="neutral" label="Working" isPulsing />;
    case 'done':
      return <StatusDot variant="success" label="Done" />;
    default:
      return <StatusDot variant="neutral" label={words(state)} />;
  }
}

/**
 * When a watcher wrote what it observed again after the task appeared (docs/api/platform.md#task):
 * the task keeps its state and summary, and its title and observation are the latest. Only an
 * open task is checked again; a done one was last changed by finishing.
 */
function watcherLine(task: TaskSummary): string | null {
  if (!task.watcher || task.watcher === 'ailism_bot' || !task.updated_at) return null;
  if (task.state !== 'needs_you' && task.state !== 'working') return null;
  const updated = Date.parse(task.updated_at);
  if (Number.isNaN(updated) || updated <= Date.parse(task.at)) return null;
  return `Watcher ${words(task.watcher)} checked again ${dateTime(task.updated_at)}`;
}

/** Who a task comes from, in a word or two. */
function from(task: TaskSummary): string {
  return task.watcher === 'ailism_bot' ? 'Ailism Bot' : (ORIGIN[task.origin] ?? words(task.origin));
}

/** A task as an AI target: people can ask about it, and the agent can point at it. */
function taskTarget(task: TaskSummary, blockId?: string): AiTarget {
  return {
    kind: 'task_item',
    label: `${task.title}${task.summary ? ` · ${task.summary}` : ''}`,
    title: task.title,
    ids: {task_id: task.id, ...(task.change_set_id ? {change_set_id: task.change_set_id} : {})},
    term: 'acom:Task',
    blockId,
  };
}

/** How the change set's checks went; a failed check is the thing to see first. */
function ChecksLine({cs}: {cs: ChangeSet}) {
  const checks = cs.checks_summary;
  if (!checks || checks.total === 0) return null;
  return (
    <StateToken
      state={checks.failed > 0 ? 'failed' : 'passed'}
      label={checks.failed > 0 ? `${checks.failed} of ${checks.total} checks failed` : `${checks.passed} of ${checks.total} checks passed`}
    />
  );
}

/** Dismiss, for a person who may: a linked change set that still waits is rejected with it. */
function DismissButton({task}: {task: TaskSummary}) {
  const router = useRouter();
  const scopes = useBoSession()?.scopes ?? [];
  if (!scopes.includes('human:task')) return null;
  return (
    <PersonButton
      action={{
        label: 'Dismiss',
        method: 'POST',
        path: `/v1/platform/tasks/${encodeURIComponent(task.id)}/dismiss`,
        confirm: `Dismiss “${task.title}”? A linked change set that is still waiting is rejected too. If the same thing happens again, a new task appears.`,
      }}
      variant="ghost"
      size="sm"
      onDone={() => router.refresh()}
    />
  );
}

/** The Ailism Bot's note on a task it had no change for ("The agent did not draft a change: …"). */
const NO_CHANGE = /^The agent did not draft a change:\s*/i;
/** A summary that only repeats the group ("Change set #1043 is waiting for your approval"). */
const ONLY_WAITING = /^Change set #\d+ is waiting/i;

/**
 * A task that needs a decision, as a card: what it is and where it came from, then either the
 * change set to review (what it changes, how its checks went) or, when the agent drafted nothing,
 * its message and a reply. A reply fits every kind of question: it puts the task in the
 * conversation for the person to answer in their own words, and any change the agent then makes
 * comes back here as a change set to approve.
 */
function DecisionCard({task, cs, blockId, intents}: {task: TaskSummary; cs?: ChangeSet; blockId?: string; intents?: string[]}) {
  const dock = useAgentDock();
  const target = taskTarget(task, blockId);
  const reply = useAiTarget(target);
  const watched = watcherLine(task);
  const csId = task.change_set_id;
  const what = cs ? (!task.summary || ONLY_WAITING.test(task.summary) ? cs.title : task.summary) : task.summary;
  const message = task.summary?.replace(NO_CHANGE, '') ?? '';

  return (
    <Card padding={4}>
      <TargetBlock target={target} intents={intents}>
        <HStack gap={2} vAlign="center" wrap="wrap">
          {task.tag ? <Token label={words(task.tag)} size="sm" /> : null}
          <StackItem size="fill">
            <Link href={`/bo/tasks/${encodeURIComponent(task.id)}`} color="primary">
              <Text weight="semibold">{task.title}</Text>
            </Link>
          </StackItem>
          <Text type="supporting" color="secondary">{`${from(task)} · ${clockFace(task.at)}`}</Text>
        </HStack>
        {csId ? (
          <div className="acom-inbox-row">
            <div className="acom-inbox-grow">
              <VStack gap={1}>
                {what ? (
                  // One line; the whole of it on hover.
                  <span title={what}>
                    <Text maxLines={1}>{cs ? `#${cs.number} · ${what}` : what}</Text>
                  </span>
                ) : null}
                <HStack gap={3} vAlign="center" wrap="wrap">
                  {cs ? <ChecksLine cs={cs} /> : null}
                  {watched ? (
                    <Text type="supporting" color="secondary">
                      {watched}
                    </Text>
                  ) : null}
                </HStack>
              </VStack>
            </div>
            <HStack gap={1} vAlign="center">
              <DismissButton task={task} />
              <Button label={cs ? `Review #${cs.number}` : 'Review'} size="sm" variant="primary" href={`/bo/change-sets/${encodeURIComponent(csId)}`} />
            </HStack>
          </div>
        ) : (
          <>
            {message ? (
              <div className="acom-inbox-ask">
                <VStack gap={0.5}>
                  <Text type="supporting" color="secondary">
                    {`${from(task)} ${message.trim().endsWith('?') ? 'asks' : 'says'}`}
                  </Text>
                  <Text>{message}</Text>
                </VStack>
              </div>
            ) : null}
            <div className="acom-inbox-row">
              <div className="acom-inbox-grow">
                <Text type="supporting" color="secondary">
                  {watched ?? 'No change drafted yet. Your answer goes to the agent; any change comes back here to approve.'}
                </Text>
              </div>
              <HStack gap={1} vAlign="center">
                <DismissButton task={task} />
                <Button
                  label="Reply"
                  size="sm"
                  variant="primary"
                  onClick={() => {
                    reply.ask();
                    dock.open();
                  }}
                />
              </HStack>
            </div>
          </>
        )}
      </TargetBlock>
    </Card>
  );
}

/**
 * A task the agent is working on, in the quiet column: its title (to the task), how far it got
 * when the summary counts ("15 of 18 checked"), and what it is doing. Dismiss is on the task's page.
 */
function WorkingItem({task, blockId, intents}: {task: TaskSummary; blockId?: string; intents?: string[]}) {
  const progress = /(\d+) of (\d+)/.exec(task.summary ?? '');
  const watched = watcherLine(task);
  return (
    <TargetBlock target={taskTarget(task, blockId)} intents={intents}>
      <HStack gap={2} vAlign="center">
        {dot('working')}
        <Link href={`/bo/tasks/${encodeURIComponent(task.id)}`} color="primary">
          <Text weight="semibold">{task.title}</Text>
        </Link>
      </HStack>
      {progress ? (
        <ProgressBar value={Number(progress[1])} max={Number(progress[2])} label={`${progress[1]} of ${progress[2]}`} isLabelHidden variant="neutral" />
      ) : null}
      <VStack gap={0.5}>
        {task.summary ? <Text>{task.summary}</Text> : null}
        <Text type="supporting" color="secondary">
          {[watched, `${from(task)} · ${clockFace(task.at)}`].filter(Boolean).join(' · ')}
        </Text>
      </VStack>
    </TargetBlock>
  );
}

export interface TaskTableProps {
  tasks: TaskSummary[];
  changeSets: Record<string, ChangeSet>;
  blockId?: string;
  intentsFor?: IntentsLookup;
}

// Two columns where there is room: the decisions, and beside them a narrower column for the
// agent's work. Below 1024 px the work column goes under the decisions.
const INBOX_CSS = `
.acom-inbox { display: grid; grid-template-columns: minmax(0, 2fr) minmax(300px, 1fr); gap: var(--spacing-5); align-items: start; }
@media (max-width: 1023px) { .acom-inbox { grid-template-columns: minmax(0, 1fr); } }
.acom-inbox-row { display: flex; align-items: flex-end; gap: var(--spacing-3); }
.acom-inbox-grow { flex: 1; min-width: 0; }
@media (max-width: 640px) { .acom-inbox-row { flex-wrap: wrap; } .acom-inbox-grow { flex-basis: 100%; } }
.acom-inbox-ask { background: var(--color-background-muted); border-radius: var(--radius-element); padding: var(--spacing-2) var(--spacing-3); }
`;

/** A group's heading: its dot, name and count, and (if any) what the group means. */
function GroupHeading({state, count}: {state: TaskState; count: number}) {
  const g = TASK_GROUPS.find(x => x.state === state);
  return (
    <HStack gap={2} vAlign="center" wrap="wrap">
      {dot(state)}
      <Text type="large" weight="semibold">
        {g?.title ?? words(state)}
      </Text>
      <Text type="supporting" color="secondary" hasTabularNumbers>
        {count}
      </Text>
      <StackItem size="fill" />
      {state === 'needs_you' && count > 0 && g?.hint ? (
        <Text type="supporting" color="secondary">
          {g.hint}
        </Text>
      ) : null}
    </HStack>
  );
}

/**
 * The inbox: on the left what needs a decision, one card each, so each reads top to bottom with
 * its one main button; on the right what the agent is working on, quieter, with how far it got.
 */
export function TaskGroups({tasks, changeSets, blockId, intentsFor}: TaskTableProps) {
  const needs = tasks.filter(t => t.state === 'needs_you');
  const working = tasks.filter(t => t.state === 'working');
  if (needs.length === 0 && working.length === 0) {
    return <EmptyState title="Nothing needs you" description="Tasks appear when you ask the agent for something, or when it notices something that needs you." />;
  }
  const workingGroup = TASK_GROUPS.find(g => g.state === 'working');
  return (
    <div className="acom-inbox">
      <style>{INBOX_CSS}</style>
      <section aria-label="Needs your decision">
        <VStack gap={3}>
          <GroupHeading state="needs_you" count={needs.length} />
          {needs.length === 0 ? (
            <Card padding={4}>
              <Text type="supporting" color="secondary">
                {TASK_GROUPS.find(g => g.state === 'needs_you')?.empty}
              </Text>
            </Card>
          ) : (
            needs.map(t => (
              <DecisionCard
                key={t.id}
                task={t}
                cs={t.change_set_id ? changeSets[t.change_set_id] : undefined}
                blockId={blockId}
                intents={intentsFor?.('task_item', t.id)}
              />
            ))
          )}
        </VStack>
      </section>
      <section aria-label="Agent is working">
        <VStack gap={3}>
          <GroupHeading state="working" count={working.length} />
          <Card padding={4}>
            <VStack gap={3}>
              {working.length === 0 ? (
                <Text type="supporting" color="secondary">
                  {workingGroup?.empty}
                </Text>
              ) : (
                working.map((t, i) => (
                  <Fragment key={t.id}>
                    {i > 0 ? <Divider /> : null}
                    <WorkingItem task={t} blockId={blockId} intents={intentsFor?.('task_item', t.id)} />
                  </Fragment>
                ))
              )}
              {working.length > 0 && workingGroup?.hint ? (
                <Text type="supporting" color="secondary">
                  {workingGroup.hint}
                </Text>
              ) : null}
            </VStack>
          </Card>
        </VStack>
      </section>
    </div>
  );
}

/**
 * One group of tasks as a compact list, for narrow places such as the conversation's canvas:
 * the state, the task (linked), where it came from, its summary and the way in. Dismiss and undo
 * stay on the inbox and the change set, where there is room to read before deciding.
 */
export function TaskList({tasks, blockId, intentsFor}: {tasks: TaskSummary[]; blockId?: string; intentsFor?: IntentsLookup}) {
  return (
    <VStack gap={3}>
      {tasks.map(task => {
        const review = task.change_set_id && task.state === 'needs_you';
        return (
          <TargetBlock
            key={task.id}
            target={{
              kind: 'task_item',
              label: `${task.title}${task.summary ? ` · ${task.summary}` : ''}`,
              ids: {task_id: task.id, ...(task.change_set_id ? {change_set_id: task.change_set_id} : {})},
              term: 'acom:Task',
              blockId,
            }}
            intents={intentsFor?.('task_item', task.id)}>
            <HStack gap={2} vAlign="start">
              <span style={{paddingTop: 6}}>{dot(task.state)}</span>
              <VStack gap={0.5}>
                <Link href={`/bo/tasks/${encodeURIComponent(task.id)}`} color="primary">
                  {task.title}
                </Link>
                <Text type="supporting" color="secondary">
                  {`${task.watcher === 'ailism_bot' ? 'Ailism Bot proposed' : (ORIGIN[task.origin] ?? words(task.origin))} · ${dateTime(task.at)}${task.tag ? ` · ${words(task.tag)}` : ''}`}
                </Text>
                {task.summary ? <Text>{task.summary}</Text> : null}
                <HStack gap={1}>
                  {review ? (
                    <Button label="Review" size="sm" href={`/bo/change-sets/${encodeURIComponent(task.change_set_id ?? '')}`} />
                  ) : (
                    <Button label="Open" size="sm" variant="ghost" href={`/bo/tasks/${encodeURIComponent(task.id)}`} />
                  )}
                </HStack>
              </VStack>
            </HStack>
          </TargetBlock>
        );
      })}
    </VStack>
  );
}

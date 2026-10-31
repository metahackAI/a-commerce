'use client';

// The operator's tasks (list_tasks), grouped as the inbox shows them: needs you, the agent is
// working, done. Each row is an AI target; its change set shows how far it got. Dismiss and undo
// are a person's decisions and go through PersonButton.

import {Fragment} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {Button} from '@astryxdesign/core/Button';
import {StatusDot} from '@astryxdesign/core/StatusDot';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';
import {VisuallyHidden} from '@astryxdesign/core/VisuallyHidden';

import {PersonButton} from '@/components/PersonButton';
import {dateTime, words} from '@/app/bo/_lib/format';
import type {ChangeSet, TaskState, TaskSummary} from '@/app/bo/_lib/types';

import {useBoSession} from './BoSession';
import {StateToken} from './StateToken';
import type {IntentsFor as IntentsLookup} from './ChangeParts';
import {AskHeaderCell, TargetBlock, TargetRow} from './targets';

export const TASK_GROUPS: {state: TaskState; title: string; empty: string}[] = [
  {state: 'needs_you', title: 'Needs your decision', empty: 'Nothing is waiting for you.'},
  {state: 'working', title: 'Agent is working', empty: 'The agent is not working on anything right now.'},
  {state: 'done', title: 'Done', empty: 'Nothing finished yet.'},
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
  if (!task.watcher || !task.updated_at) return null;
  if (task.state !== 'needs_you' && task.state !== 'working') return null;
  const updated = Date.parse(task.updated_at);
  if (Number.isNaN(updated) || updated <= Date.parse(task.at)) return null;
  return `Watcher ${words(task.watcher)} checked again ${dateTime(task.updated_at)}`;
}

/** The line about a task's change set: its state and how its checks went. */
function changeSetLine(cs: Pick<ChangeSet, 'number' | 'state' | 'version' | 'checks_summary'> | undefined) {
  if (!cs) return null;
  const checks = cs.checks_summary;
  return (
    <HStack gap={3} vAlign="center" wrap="wrap">
      <StateToken state={cs.state} />
      {checks && checks.total > 0 ? (
        <StateToken
          state={checks.failed > 0 ? 'failed' : 'passed'}
          label={checks.failed > 0 ? `${checks.failed} of ${checks.total} checks failed` : `${checks.passed} of ${checks.total} checks passed`}
        />
      ) : null}
    </HStack>
  );
}

export interface TaskTableProps {
  tasks: TaskSummary[];
  changeSets: Record<string, ChangeSet>;
  blockId?: string;
  intentsFor?: IntentsLookup;
}

function TaskRow({task, cs, blockId, intents}: {task: TaskSummary; cs?: ChangeSet; blockId?: string; intents?: string[]}) {
  const router = useRouter();
  const session = useBoSession();
  const scopes = session?.scopes ?? [];
  const open = task.state !== 'done' && task.state !== 'dismissed';
  const canDismiss = open && scopes.includes('human:task');
  const canUndo = task.state === 'done' && cs?.state === 'live' && cs.undo_until !== null && scopes.includes('human:undo');
  const review = task.change_set_id && (task.state === 'needs_you' || cs?.state === 'waiting_for_approval');
  const watched = watcherLine(task);

  return (
    <TargetRow
      target={{
        kind: 'task_item',
        label: `${task.title}${task.summary ? ` · ${task.summary}` : ''}`,
        ids: {task_id: task.id, ...(task.change_set_id ? {change_set_id: task.change_set_id} : {})},
        term: 'acom:Task',
        blockId,
      }}
      intents={intents}>
      <TableCell>
        <VStack gap={0.5}>
          <Link href={`/bo/tasks/${encodeURIComponent(task.id)}`} color="primary">
            {task.title}
          </Link>
          <Text type="supporting" color="secondary">
            {`${ORIGIN[task.origin] ?? words(task.origin)} · ${dateTime(task.at)}${task.tag ? ` · ${words(task.tag)}` : ''}`}
          </Text>
        </VStack>
      </TableCell>
      <TableCell>
        <VStack gap={1}>
          {task.summary ? <Text>{task.summary}</Text> : null}
          {watched ? (
            <Text type="supporting" color="secondary">
              {watched}
            </Text>
          ) : null}
          {changeSetLine(cs)}
        </VStack>
      </TableCell>
      <TableCell style={{maxWidth: 'none'}}>
        <HStack gap={1} hAlign="end">
          {review ? (
            <Button label="Review" size="sm" href={`/bo/change-sets/${encodeURIComponent(task.change_set_id ?? '')}`} />
          ) : task.change_set_id ? (
            <Button label="Change set" size="sm" variant="ghost" href={`/bo/change-sets/${encodeURIComponent(task.change_set_id)}`} />
          ) : (
            <Button label="Open" size="sm" variant="ghost" href={`/bo/tasks/${encodeURIComponent(task.id)}`} />
          )}
          {canUndo && cs ? (
            <PersonButton
              action={{
                label: 'Undo',
                method: 'POST',
                path: `/v1/platform/change-sets/${encodeURIComponent(cs.id)}/undo`,
                confirm: `Undo every change of #${cs.number} in reverse order?${cs.undo_until ? ` Undo is open until ${dateTime(cs.undo_until)}.` : ''}`,
              }}
              variant="secondary"
              size="sm"
              onDone={() => router.refresh()}
            />
          ) : null}
          {canDismiss ? (
            <PersonButton
              action={{
                label: 'Dismiss',
                method: 'POST',
                path: `/v1/platform/tasks/${encodeURIComponent(task.id)}/dismiss`,
                confirm: `Dismiss “${task.title}”? A linked change set that is still waiting is rejected too. If the same thing happens again, a new task appears.`,
              }}
              variant="secondary"
              size="sm"
              onDone={() => router.refresh()}
            />
          ) : null}
        </HStack>
      </TableCell>
    </TargetRow>
  );
}

/** Shrink a column to its content (the buttons), so the text columns get the width. */
const FIT = {width: '1%'} as const;

/**
 * Tasks in their groups, as one table so the columns line up from group to group: each group is
 * a row with its name and count, then its tasks (as Linear groups a list by status). The group
 * says the tasks' state, so the rows don't repeat it.
 */
export function TaskGroups({
  tasks,
  changeSets,
  states = ['needs_you', 'working', 'done'],
  blockId,
  intentsFor,
}: TaskTableProps & {states?: TaskState[]}) {
  if (tasks.length === 0) {
    return <EmptyState title="No tasks" description="Tasks appear when you ask the agent for something, or when it notices something that needs you." />;
  }
  return (
    <Table density="compact" verticalAlign="top" hasHover>
      <TableHeader>
        <TableRow isHeaderRow>
          <TableHeaderCell style={{width: '50%'}}>Task</TableHeaderCell>
          <TableHeaderCell style={{width: '50%'}}>Where it is</TableHeaderCell>
          <TableHeaderCell style={FIT}>
            <VisuallyHidden>Actions</VisuallyHidden>
          </TableHeaderCell>
          <AskHeaderCell />
        </TableRow>
      </TableHeader>
      <TableBody>
        {TASK_GROUPS.filter(g => states.includes(g.state)).map(g => {
          const group = tasks.filter(t => t.state === g.state);
          return (
            <Fragment key={g.state}>
              <TableRow>
                <TableCell colSpan={4}>
                  <VStack gap={0.5} paddingBlockStart={2}>
                    <HStack gap={2} vAlign="center">
                      <Text type="label" weight="semibold">
                        {g.title}
                      </Text>
                      <Text type="supporting" color="secondary">
                        {group.length}
                      </Text>
                    </HStack>
                    {group.length === 0 ? (
                      <Text type="supporting" color="secondary">
                        {g.empty}
                      </Text>
                    ) : null}
                  </VStack>
                </TableCell>
              </TableRow>
              {group.map(t => (
                <TaskRow
                  key={t.id}
                  task={t}
                  cs={t.change_set_id ? changeSets[t.change_set_id] : undefined}
                  blockId={blockId}
                  intents={intentsFor?.('task_item', t.id)}
                />
              ))}
            </Fragment>
          );
        })}
      </TableBody>
    </Table>
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
                  {`${ORIGIN[task.origin] ?? words(task.origin)} · ${dateTime(task.at)}${task.tag ? ` · ${words(task.tag)}` : ''}`}
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

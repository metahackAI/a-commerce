'use client';

// The operator's tasks (list_tasks), grouped as the inbox shows them: needs you, the agent is
// working, done. Each row is an AI target; its change set shows how far it got. Dismiss and undo
// are a person's decisions and go through PersonButton.

import {useRouter} from 'next/navigation';
import {Card} from '@astryxdesign/core/Card';
import {HStack, StackItem, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {Button} from '@astryxdesign/core/Button';
import {StatusDot} from '@astryxdesign/core/StatusDot';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {Table, TableBody, TableCell} from '@astryxdesign/core/Table';
import {useMediaQuery} from '@astryxdesign/core/hooks';

import {PersonButton} from '@/components/PersonButton';
import {dateTime, words} from '@/app/bo/_lib/format';
import type {ChangeSet, TaskState, TaskSummary} from '@/app/bo/_lib/types';

import {useBoSession} from './BoSession';
import {StateToken} from './StateToken';
import type {IntentsFor as IntentsLookup} from './ChangeParts';
import {TargetBlock, TargetRow} from './targets';

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

/**
 * The line about a task's change set: its state and how its checks went. Under "Needs your
 * decision" that it is waiting for approval goes without saying.
 */
function changeSetLine(cs: Pick<ChangeSet, 'number' | 'state' | 'version' | 'checks_summary'> | undefined, group: TaskState) {
  if (!cs) return null;
  const checks = cs.checks_summary;
  return (
    <HStack gap={3} vAlign="center" wrap="wrap">
      {group === 'needs_you' && cs.state === 'waiting_for_approval' ? null : <StateToken state={cs.state} />}
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

/** The buttons' column, as wide in every group so the columns line up from one to the next. */
const ACTIONS = {width: 190, minWidth: 190, maxWidth: 'none'} as const;

/** Below this width the inbox stacks each task in one column: a phone has no room for three. */
const NARROW = '(max-width: 640px)';

function TaskRow({
  task,
  cs,
  blockId,
  intents,
  isNarrow,
}: {
  task: TaskSummary;
  cs?: ChangeSet;
  blockId?: string;
  intents?: string[];
  isNarrow: boolean;
}) {
  const router = useRouter();
  const session = useBoSession();
  const scopes = session?.scopes ?? [];
  const open = task.state !== 'done' && task.state !== 'dismissed';
  const canDismiss = open && scopes.includes('human:task');
  const canUndo = task.state === 'done' && cs?.state === 'live' && cs.undo_until !== null && scopes.includes('human:undo');
  const review = task.change_set_id && (task.state === 'needs_you' || cs?.state === 'waiting_for_approval');
  const watched = watcherLine(task);

  const what = (
    <VStack gap={0.5}>
      <Link href={`/bo/tasks/${encodeURIComponent(task.id)}`} color="primary">
        <Text weight="semibold">{task.title}</Text>
      </Link>
      <Text type="supporting" color="secondary">
        {`${task.watcher === 'ailism_bot' ? 'Ailism Bot proposed' : (ORIGIN[task.origin] ?? words(task.origin))} · ${dateTime(task.at)}${task.tag ? ` · ${words(task.tag)}` : ''}`}
      </Text>
    </VStack>
  );
  const where = (
    <VStack gap={1}>
      {task.summary ? <Text>{task.summary}</Text> : null}
      {watched ? (
        <Text type="supporting" color="secondary">
          {watched}
        </Text>
      ) : null}
      {changeSetLine(cs, task.state)}
    </VStack>
  );
  const actions = (
    <HStack gap={1} hAlign={isNarrow ? 'start' : 'end'} wrap="wrap">
      {review ? (
        <Button label="Review" size="sm" variant="primary" href={`/bo/change-sets/${encodeURIComponent(task.change_set_id ?? '')}`} />
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
          variant="ghost"
          size="sm"
          onDone={() => router.refresh()}
        />
      ) : null}
    </HStack>
  );

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
      {isNarrow ? (
        <TableCell>
          <VStack gap={2}>
            {what}
            {where}
            {actions}
          </VStack>
        </TableCell>
      ) : (
        <>
          <TableCell style={{width: '44%'}}>{what}</TableCell>
          <TableCell>{where}</TableCell>
          <TableCell style={ACTIONS}>{actions}</TableCell>
        </>
      )}
    </TargetRow>
  );
}

/**
 * Tasks in their groups, each group its own card so what needs a person stands apart from what the
 * agent is doing and what is finished: a dot, the group's name and count, what it means, then its
 * tasks. The group says the tasks' state, so the rows don't repeat it. Finished work sits on a muted
 * card, the quietest thing on the page.
 */
export function TaskGroups({
  tasks,
  changeSets,
  states = ['needs_you', 'working', 'done'],
  blockId,
  intentsFor,
}: TaskTableProps & {states?: TaskState[]}) {
  const isNarrow = useMediaQuery(NARROW);
  if (tasks.length === 0) {
    return <EmptyState title="No tasks" description="Tasks appear when you ask the agent for something, or when it notices something that needs you." />;
  }
  return (
    <VStack gap={4}>
      {TASK_GROUPS.filter(g => states.includes(g.state)).map(g => {
        const group = tasks.filter(t => t.state === g.state);
        return (
          <Card key={g.state} variant={g.state === 'needs_you' || g.state === 'working' ? 'default' : 'muted'}>
            <VStack gap={3}>
              <HStack gap={2} vAlign="center" wrap="wrap">
                {dot(g.state)}
                <Text type="large" weight="semibold">
                  {g.title}
                </Text>
                <Text type="supporting" color="secondary" hasTabularNumbers>
                  {group.length}
                </Text>
                <StackItem size="fill" />
                {g.hint && group.length > 0 && !isNarrow ? (
                  <Text type="supporting" color="secondary">
                    {g.hint}
                  </Text>
                ) : null}
              </HStack>
              {group.length === 0 ? (
                <Text type="supporting" color="secondary">
                  {g.empty}
                </Text>
              ) : (
                <Table aria-label={g.title} verticalAlign="top" hasHover>
                  <TableBody>
                    {group.map(t => (
                      <TaskRow
                        key={t.id}
                        task={t}
                        cs={t.change_set_id ? changeSets[t.change_set_id] : undefined}
                        blockId={blockId}
                        intents={intentsFor?.('task_item', t.id)}
                        isNarrow={isNarrow}
                      />
                    ))}
                  </TableBody>
                </Table>
              )}
            </VStack>
          </Card>
        );
      })}
    </VStack>
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

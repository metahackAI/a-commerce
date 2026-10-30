import {HomeView} from '@/components/bo/HomeView';

import {param, read, readOptional, type SearchParams} from '../_lib/server';
import type {ChangeSet, Page, TaskSummary} from '../_lib/types';

export const metadata = {title: 'Inbox'};

// Home (B5): the operator's tasks from list_tasks, each with the state of its change set
// (get_change_set, top-level fields only), and the agent workspace.
export default async function BackOfficeHome({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const tasks = await read<Page<TaskSummary>>('/v1/platform/tasks', {state: ['needs_you', 'working', 'done'], limit: 100});

  const changeSets: Record<string, ChangeSet> = {};
  if (tasks.ok) {
    const ids = [...new Set(tasks.data.items.map(t => t.change_set_id).filter((x): x is string => Boolean(x)))];
    const found = await Promise.all(
      ids.map(id =>
        readOptional<ChangeSet>(`/v1/platform/change-sets/${encodeURIComponent(id)}`, {
          fields: ['id', 'number', 'title', 'state', 'version', 'undo_until', 'checks_summary', 'needs_approval'],
        }),
      ),
    );
    for (const cs of found) if (cs) changeSets[cs.id] = cs;
  }

  return (
    <HomeView
      tasks={tasks.ok ? tasks.data.items : []}
      total={tasks.ok ? tasks.data.total : 0}
      changeSets={changeSets}
      error={tasks.ok ? null : tasks.error}
      initialView={param(sp, 'view') === 'agent' ? 'agent' : 'inbox'}
    />
  );
}

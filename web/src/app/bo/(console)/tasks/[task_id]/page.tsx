import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';
import {TaskView} from '@/components/bo/TaskView';

import {read, readOptional} from '../../../_lib/server';
import type {ChangeSet, Task} from '../../../_lib/types';

export const metadata = {title: 'Task'};

// One task (get_task) and, when it has one, its change set's state (get_change_set).
export default async function TaskPage({params}: {params: Promise<{task_id: string}>}) {
  const {task_id: id} = await params;
  const task = await read<Task>(`/v1/platform/tasks/${encodeURIComponent(id)}`);
  if (!task.ok) {
    return (
      <PageFrame back={{href: '/bo', label: 'Inbox'}} title={id}>
        <ErrorState error={task.error} title="This task could not be loaded." />
      </PageFrame>
    );
  }
  const csId = task.data.related?.change_set_id;
  const changeSet = csId
    ? await readOptional<ChangeSet>(`/v1/platform/change-sets/${encodeURIComponent(csId)}`, {fields: ['id', 'number', 'title', 'state', 'version']})
    : null;
  return <TaskView task={task.data} changeSet={changeSet} />;
}

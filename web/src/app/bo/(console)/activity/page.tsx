import {ActionLogTable} from '@/components/bo/ActionLogTable';
import {ErrorState} from '@/components/bo/ErrorState';
import {FilterBar} from '@/components/bo/FilterBar';
import {PageFrame} from '@/components/bo/PageFrame';
import {Pager} from '@/components/bo/Pager';

import {endOfDay, param, read, type SearchParams} from '../../_lib/server';
import type {ActionEntry, Page} from '../../_lib/types';

export const metadata = {title: 'Activity'};

// Activity (search_action_log): every tool call, preview, approval and run, by who, which tool,
// which record, which change set, how it ended, and when.
export default async function ActivityPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const values = {
    agent: param(sp, 'agent'),
    instrument: param(sp, 'instrument'),
    object_id: param(sp, 'object_id'),
    change_set_id: param(sp, 'change_set_id'),
    action_status: param(sp, 'action_status'),
    start_time_from: param(sp, 'start_time_from'),
    start_time_through: param(sp, 'start_time_through'),
  };
  const cursor = param(sp, 'cursor');
  const list = await read<Page<ActionEntry>>('/v1/platform/action-logs', {
    ...values,
    start_time_through: endOfDay(values.start_time_through),
    limit: 50,
    cursor,
  });

  return (
    <PageFrame
      title="Activity"
      description="The action log: everything the agents, people and the system did in this store. It is only ever added to."
      width={1400}
      toolbar={
        <FilterBar
          values={values}
          filters={[
            {kind: 'text', key: 'agent', label: 'Who', placeholder: 'adm_sj or a model name', width: 200},
            {kind: 'text', key: 'instrument', label: 'Tool', placeholder: 'e.g. set_sale_prices', width: 180},
            {kind: 'text', key: 'object_id', label: 'Record', placeholder: 'e.g. prd_windbreaker', width: 180},
            {kind: 'text', key: 'change_set_id', label: 'Change set', placeholder: 'e.g. cs_1042', width: 140},
            {
              kind: 'select',
              key: 'action_status',
              label: 'Status',
              options: [
                {value: 'potential', label: 'Potential (preview)'},
                {value: 'active', label: 'Active'},
                {value: 'completed', label: 'Completed'},
                {value: 'failed', label: 'Failed'},
              ],
            },
            {kind: 'date', key: 'start_time_from', label: 'From'},
            {kind: 'date', key: 'start_time_through', label: 'Through'},
          ]}
        />
      }>
      {list.ok ? (
        <>
          <ActionLogTable entries={list.data.items} empty="No actions match" />
          <Pager base="/bo/activity" params={{...values, cursor}} shown={list.data.items.length} total={list.data.total} nextCursor={list.data.next_cursor} noun="actions" />
        </>
      ) : (
        <ErrorState error={list.error} title="The activity could not be loaded." />
      )}
    </PageFrame>
  );
}

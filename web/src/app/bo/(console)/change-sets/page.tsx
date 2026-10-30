import {FilterBar} from '@/components/bo/FilterBar';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';
import {Pager} from '@/components/bo/Pager';
import {ChangeSetList} from '@/components/bo/lists/platform';

import {param, params, read, type SearchParams} from '../../_lib/server';
import type {ChangeSetSummary, Page} from '../../_lib/types';

export const metadata = {title: 'Change sets'};

const STATES = ['drafting', 'waiting_for_approval', 'running', 'verifying', 'live', 'failed', 'rejected', 'undone'];

const label = (s: string) => s.charAt(0).toUpperCase() + s.slice(1).replace(/_/g, ' ');

// Change sets (search_change_sets): filter by state, who asked, when, and whether they can still
// be undone.
export default async function ChangeSetsPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const values = {
    state: params(sp, 'state'),
    created_by: param(sp, 'created_by'),
    created_from: param(sp, 'created_from'),
    created_through: param(sp, 'created_through'),
    undoable_only: param(sp, 'undoable_only'),
    sort: param(sp, 'sort'),
  };
  const cursor = param(sp, 'cursor');
  const list = await read<Page<ChangeSetSummary>>('/v1/platform/change-sets', {
    state: values.state,
    created_by: values.created_by,
    created_from: values.created_from,
    created_through: values.created_through ? `${values.created_through}T23:59` : undefined,
    undoable_only: values.undoable_only === 'true' ? true : undefined,
    sort: values.sort,
    limit: 50,
    cursor,
  });

  return (
    <PageFrame
      title="Change sets"
      description="Every write the agent prepares is a change set. Nothing in it runs until a person approves it."
      toolbar={
        <FilterBar
          values={values}
          filters={[
            {kind: 'multi', key: 'state', label: 'State', options: STATES.map(s => ({value: s, label: label(s)}))},
            {kind: 'text', key: 'created_by', label: 'Requested by', placeholder: 'adm_… or sys_…'},
            {kind: 'date', key: 'created_from', label: 'Created from'},
            {kind: 'date', key: 'created_through', label: 'Created through'},
            {kind: 'select', key: 'undoable_only', label: 'Undo', options: [{value: 'true', label: 'Can still be undone'}]},
            {
              kind: 'select',
              key: 'sort',
              label: 'Sort',
              placeholder: 'Newest first',
              options: [
                {value: '-created_at', label: 'Newest first'},
                {value: 'created_at', label: 'Oldest first'},
                {value: '-number', label: 'Highest number first'},
              ],
            },
          ]}
        />
      }>
      {list.ok ? (
        <>
          <ChangeSetList items={list.data.items} />
          <Pager base="/bo/change-sets" params={{...values, cursor}} shown={list.data.items.length} total={list.data.total} nextCursor={list.data.next_cursor} noun="change sets" />
        </>
      ) : (
        <ErrorState error={list.error} title="The change sets could not be loaded." />
      )}
    </PageFrame>
  );
}

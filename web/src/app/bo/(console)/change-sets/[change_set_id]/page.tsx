import {ChangeSetView} from '@/components/bo/ChangeSetView';
import {PageFrame} from '@/components/bo/PageFrame';
import {ErrorState} from '@/components/bo/ErrorState';

import {param, read, readOptional, type SearchParams} from '../../../_lib/server';
import type {ActionEntry, ChangeSet, Page, Verification} from '../../../_lib/types';

export const metadata = {title: 'Change set'};

const RAN = new Set(['running', 'verifying', 'live', 'failed', 'undone']);

// One change set: get_change_set (this or an earlier version), verify_change_set once it has
// run, and its history from search_action_log.
export default async function ChangeSetPage({
  params,
  searchParams,
}: {
  params: Promise<{change_set_id: string}>;
  searchParams: SearchParams;
}) {
  const {change_set_id: id} = await params;
  const sp = await searchParams;
  const path = `/v1/platform/change-sets/${encodeURIComponent(id)}`;
  const version = param(sp, 'version');

  const shown = await read<ChangeSet>(path, version ? {version} : undefined);
  if (!shown.ok) {
    return (
      <PageFrame back={{href: '/bo/change-sets', label: 'Change sets'}} title={id}>
        <ErrorState error={shown.error} title="This change set could not be loaded." />
      </PageFrame>
    );
  }
  const cs = shown.data;

  const [latest, verification, history] = await Promise.all([
    version ? readOptional<Pick<ChangeSet, 'version'>>(path, {fields: ['version']}) : Promise.resolve({version: cs.version}),
    RAN.has(cs.state) ? readOptional<Verification>(`${path}/verification`) : Promise.resolve(null),
    read<Page<ActionEntry>>('/v1/platform/action-logs', {change_set_id: id, limit: 100}),
  ]);

  return (
    <ChangeSetView
      changeSet={cs}
      latestVersion={latest?.version ?? cs.version}
      verification={verification}
      history={history.ok ? history.data.items : null}
      historyError={history.ok ? null : history.error}
    />
  );
}

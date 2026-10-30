import {ActionView} from '@/components/bo/ActionView';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';

import {read} from '../../../_lib/server';
import type {ActionEntry} from '../../../_lib/types';

export const metadata = {title: 'Action'};

// One action from the log (get_action_log).
export default async function ActionPage({params}: {params: Promise<{action_id: string}>}) {
  const {action_id: id} = await params;
  const action = await read<ActionEntry>(`/v1/platform/action-logs/${encodeURIComponent(id)}`);
  if (!action.ok) {
    return (
      <PageFrame back={{href: '/bo/activity', label: 'Activity'}} title={id}>
        <ErrorState error={action.error} title="This action could not be loaded." />
      </PageFrame>
    );
  }
  return <ActionView action={action.data} />;
}

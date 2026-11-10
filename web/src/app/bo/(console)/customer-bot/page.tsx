import {CustomerBotDashboard} from '@/components/bo/customer-bot/CustomerBotDashboard';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';

import {read} from '../../_lib/server';
import type {CustomerBotRun, CustomerBotRunDetail, CustomerBotSession, CustomerBotStatus, Page} from '../../_lib/types';

export const metadata = {title: 'Ailism Customer Bot'};

// Ailism Customer Bot's dashboard (docs/api/platform.md#customer_bot): get_customer_bot_status,
// the run at work with its sessions, recent runs and recent sessions.
export default async function CustomerBotPage() {
  const status = await read<CustomerBotStatus>('/v1/platform/customer-bot');
  if (!status.ok) {
    return (
      <PageFrame title="Ailism Customer Bot">
        <ErrorState error={status.error} title="The customer bot could not be read." />
      </PageFrame>
    );
  }
  const workingId = status.data.working?.id;
  const [working, runs, sessions] = await Promise.all([
    workingId ? read<CustomerBotRunDetail>(`/v1/platform/customer-bot/runs/${encodeURIComponent(workingId)}`) : null,
    read<Page<CustomerBotRun>>('/v1/platform/customer-bot/runs', {limit: 8}),
    read<Page<CustomerBotSession>>('/v1/platform/customer-bot/sessions', {limit: 10}),
  ]);
  return (
    <CustomerBotDashboard
      status={status.data}
      working={working?.ok ? working.data : null}
      runs={runs.ok ? runs.data.items : []}
      sessions={sessions.ok ? sessions.data.items : []}
    />
  );
}

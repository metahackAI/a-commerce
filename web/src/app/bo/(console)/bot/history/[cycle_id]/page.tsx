import {BotCycleView} from '@/components/bo/bot/BotHistory';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';

import {read} from '../../../../_lib/server';
import type {BotCycle} from '../../../../_lib/types';

export const metadata = {title: 'Ailism Bot cycle'};

// One cycle in full (get_bot_cycle).
export default async function BotCyclePage({params}: {params: Promise<{cycle_id: string}>}) {
  const {cycle_id: id} = await params;
  const cycle = await read<BotCycle>(`/v1/platform/bot/cycles/${encodeURIComponent(id)}`);
  if (!cycle.ok) {
    return (
      <PageFrame back={{href: '/bo/bot/history', label: 'History'}} title={id}>
        <ErrorState error={cycle.error} title="This cycle could not be loaded." />
      </PageFrame>
    );
  }
  return <BotCycleView cycle={cycle.data} />;
}

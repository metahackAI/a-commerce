import {BotDashboard} from '@/components/bo/bot/BotDashboard';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';

import {read} from '../../_lib/server';
import type {BotCycleSummary, BotDecision, BotFeedback, BotStatus, Page} from '../../_lib/types';

export const metadata = {title: 'Ailism Bot'};

// Ailism Bot's dashboard (docs/api/platform.md#bot): get_bot_status, the decisions waiting for a
// person, open feedback, and the recent cycles.
export default async function BotPage() {
  const [status, waiting, cycles, feedback] = await Promise.all([
    read<BotStatus>('/v1/platform/bot'),
    read<Page<BotDecision>>('/v1/platform/bot/decisions', {state: ['waiting', 'needs_you', 'drafting'], limit: 6}),
    read<Page<BotCycleSummary>>('/v1/platform/bot/cycles', {limit: 6}),
    read<Page<BotFeedback>>('/v1/platform/bot/feedback', {state: 'open', limit: 20}),
  ]);
  if (!status.ok) {
    return (
      <PageFrame title="Ailism Bot">
        <ErrorState error={status.error} title="The bot could not be read." />
      </PageFrame>
    );
  }
  return (
    <BotDashboard
      status={status.data}
      waiting={waiting.ok ? waiting.data.items : []}
      cycles={cycles.ok ? cycles.data.items : []}
      feedback={feedback.ok ? feedback.data.items : []}
    />
  );
}

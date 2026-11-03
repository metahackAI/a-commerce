import {BotSettingsForm} from '@/components/bo/bot/BotSettingsForm';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';

import {read} from '../../../_lib/server';
import type {BotStatus} from '../../../_lib/types';

export const metadata = {title: 'Ailism Bot settings'};

// Ailism Bot's settings (get_bot_status; saved with the person-only PUT /v1/platform/bot/settings).
export default async function BotSettingsPage() {
  const status = await read<BotStatus>('/v1/platform/bot');
  if (!status.ok) {
    return (
      <PageFrame back={{href: '/bo/bot', label: 'Ailism Bot'}} title="Settings">
        <ErrorState error={status.error} title="The settings could not be read." />
      </PageFrame>
    );
  }
  return <BotSettingsForm status={status.data} />;
}

import {CustomerBotSettingsForm} from '@/components/bo/customer-bot/CustomerBotSettingsForm';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';

import {read} from '../../../_lib/server';
import type {CustomerBotStatus} from '../../../_lib/types';

export const metadata = {title: 'Customer bot settings'};

// Ailism Customer Bot's settings, from get_customer_bot_status.
export default async function CustomerBotSettingsPage() {
  const status = await read<CustomerBotStatus>('/v1/platform/customer-bot');
  if (!status.ok) {
    return (
      <PageFrame title="Settings" back={{href: '/bo/customer-bot', label: 'Ailism Customer Bot'}}>
        <ErrorState error={status.error} title="The settings could not be read." />
      </PageFrame>
    );
  }
  return <CustomerBotSettingsForm status={status.data} />;
}

import {SessionDetail} from '@/components/bo/customer-bot/SessionDetail';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';

import {read} from '../../../../_lib/server';
import type {CustomerBotSessionDetail} from '../../../../_lib/types';

export const metadata = {title: 'Customer bot session'};

// One shopping session whole (get_customer_bot_session).
export default async function CustomerBotSessionPage({params}: {params: Promise<{session_id: string}>}) {
  const {session_id: id} = await params;
  const session = await read<CustomerBotSessionDetail>(`/v1/platform/customer-bot/sessions/${encodeURIComponent(id)}`);
  if (!session.ok) {
    return (
      <PageFrame title="Session" back={{href: '/bo/customer-bot/sessions', label: 'Sessions'}}>
        <ErrorState error={session.error} title="This session could not be read." />
      </PageFrame>
    );
  }
  return <SessionDetail session={session.data} />;
}

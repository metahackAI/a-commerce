import {HStack} from '@astryxdesign/core/Layout';
import {Button} from '@astryxdesign/core/Button';
import {Text} from '@astryxdesign/core/Text';

import {CustomerBotTabs, SessionTable} from '@/components/bo/customer-bot/parts';
import {ErrorState} from '@/components/bo/ErrorState';
import {PageFrame} from '@/components/bo/PageFrame';

import {read} from '../../../_lib/server';
import type {CustomerBotSession, Page} from '../../../_lib/types';

export const metadata = {title: 'Customer bot sessions'};

const STATES = ['running', 'done', 'failed'];

// Every shopping session (list_customer_bot_sessions), newest first; by state or by member.
export default async function CustomerBotSessionsPage({searchParams}: {searchParams: Promise<{state?: string; customer_id?: string; page?: string}>}) {
  const q = await searchParams;
  const state = q.state && STATES.includes(q.state) ? q.state : undefined;
  const page = Math.max(1, Number(q.page) || 1);
  const limit = 25;
  const sessions = await read<Page<CustomerBotSession>>('/v1/platform/customer-bot/sessions', {
    state,
    customer_id: q.customer_id,
    limit,
    cursor: page > 1 ? `o${(page - 1) * limit}` : undefined,
  });
  const link = (s?: string, p?: number) =>
    `/bo/customer-bot/sessions?${new URLSearchParams({...(s ? {state: s} : {}), ...(q.customer_id ? {customer_id: q.customer_id} : {}), ...(p && p > 1 ? {page: String(p)} : {})}).toString()}`;
  return (
    <PageFrame
      title="Sessions"
      back={{href: '/bo/customer-bot', label: 'Ailism Customer Bot'}}
      description={q.customer_id ? `The sessions that shopped as ${q.customer_id}.` : 'Every visit a shopper made, newest first. Open one to see each step it took.'}
      toolbar={<CustomerBotTabs value="sessions" />}>
      <HStack gap={1} wrap="wrap">
        <Button label="All" size="sm" variant={state ? 'ghost' : 'secondary'} href={link()} />
        <Button label="Shopping now" size="sm" variant={state === 'running' ? 'secondary' : 'ghost'} href={link('running')} />
        <Button label="Done" size="sm" variant={state === 'done' ? 'secondary' : 'ghost'} href={link('done')} />
        <Button label="Stopped early" size="sm" variant={state === 'failed' ? 'secondary' : 'ghost'} href={link('failed')} />
      </HStack>
      {sessions.ok ? (
        <>
          <SessionTable sessions={sessions.data.items} />
          <HStack gap={2} vAlign="center">
            <Text type="supporting" color="secondary">{`${sessions.data.total} sessions`}</Text>
            {page > 1 ? <Button label="Newer" size="sm" variant="ghost" href={link(state, page - 1)} /> : null}
            {page * limit < sessions.data.total ? <Button label="Older" size="sm" variant="ghost" href={link(state, page + 1)} /> : null}
          </HStack>
        </>
      ) : (
        <ErrorState error={sessions.error} title="The sessions could not be read." />
      )}
    </PageFrame>
  );
}

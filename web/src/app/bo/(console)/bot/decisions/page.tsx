import {BotDecisions} from '@/components/bo/bot/BotDecisions';

import {DECISION_VIEWS} from '../../../_lib/bot';

import {param, read, type SearchParams} from '../../../_lib/server';
import type {BotDecision, BotStatus, Page} from '../../../_lib/types';

export const metadata = {title: 'AI decisions'};

// AI decisions (list_bot_decisions), by view; `?decision=` opens one (get_bot_decision, with its
// feedback) at the top.
export default async function BotDecisionsPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const view = DECISION_VIEWS.find(v => v.value === param(sp, 'view')) ?? DECISION_VIEWS[0];
  const cursor = param(sp, 'cursor');
  const one = param(sp, 'decision');
  const [list, status, single] = await Promise.all([
    read<Page<BotDecision>>('/v1/platform/bot/decisions', {state: view.states, limit: 20, ...(cursor ? {cursor} : {})}),
    read<BotStatus>('/v1/platform/bot'),
    one ? read<BotDecision>(`/v1/platform/bot/decisions/${encodeURIComponent(one)}`) : Promise.resolve(null),
  ]);
  const items = list.ok ? list.data.items : [];
  const first = single && single.ok ? [single.data] : [];
  return (
    <BotDecisions
      view={one ? 'all' : view.value}
      items={[...first, ...items.filter(d => !first.some(f => f.id === d.id))]}
      total={list.ok ? list.data.total : 0}
      params={{view: view.value, ...(cursor ? {cursor} : {})}}
      nextCursor={list.ok ? (list.data.next_cursor ?? null) : null}
      error={list.ok ? null : list.error}
      isOff={status.ok ? status.data.settings.mode === 'off' : false}
    />
  );
}

import {BotHistory} from '@/components/bo/bot/BotHistory';

import {param, read, type SearchParams} from '../../../_lib/server';
import type {BotCycleSummary, BotStatus, Page} from '../../../_lib/types';

export const metadata = {title: 'Ailism Bot history'};

// Every cycle Ailism Bot ran (list_bot_cycles).
export default async function BotHistoryPage({searchParams}: {searchParams: SearchParams}) {
  const sp = await searchParams;
  const cursor = param(sp, 'cursor');
  const [list, status] = await Promise.all([
    read<Page<BotCycleSummary>>('/v1/platform/bot/cycles', {limit: 20, ...(cursor ? {cursor} : {})}),
    read<BotStatus>('/v1/platform/bot'),
  ]);
  return (
    <BotHistory
      items={list.ok ? list.data.items : []}
      total={list.ok ? list.data.total : 0}
      params={cursor ? {cursor} : {}}
      nextCursor={list.ok ? (list.data.next_cursor ?? null) : null}
      error={list.ok ? null : list.error}
      isOff={status.ok ? status.data.settings.mode === 'off' : false}
    />
  );
}

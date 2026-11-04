'use client';

// AI decisions (list_bot_decisions): what Ailism Bot proposed, grouped by where the change is —
// waiting for a person, done, turned down — each with its reason, evidence and expected effect,
// and the person's buttons (review the change set, ask again, feedback).

import {Text} from '@astryxdesign/core/Text';
import {VStack} from '@astryxdesign/core/Layout';

import type {ApiErrorBody} from '@/lib/api';
import {DECISION_VIEWS} from '@/app/bo/_lib/bot';
import type {BotDecision} from '@/app/bo/_lib/types';

import {ErrorState} from '../ErrorState';
import {NavTabs} from '../NavTabs';
import {PageFrame} from '../PageFrame';
import {Pager} from '../Pager';
import {BotTabs, DecisionCard, RunNowButton, useBotEvents} from './parts';


export function BotDecisions({
  view,
  items,
  total,
  params,
  nextCursor,
  error,
  isOff,
  counts,
}: {
  counts: Record<string, number>;
  view: string;
  items: BotDecision[];
  total: number;
  params: Record<string, string | string[] | undefined>;
  nextCursor: string | null;
  error: ApiErrorBody | null;
  isOff: boolean;
}) {
  useBotEvents();
  const current = DECISION_VIEWS.find(v => v.value === view) ?? DECISION_VIEWS[0];
  return (
    <PageFrame
      title="AI decisions"
      back={{href: '/bo/bot', label: 'Ailism Bot'}}
      description="Each proposal of the bot became a task, and the operations agent drafts it as a change set. Review it there to approve or reject; ask again with a correction, or leave feedback the bot reads next time."
      actions={<RunNowButton isOff={isOff} />}
      toolbar={
        <VStack gap={2}>
          <BotTabs value="decisions" />
          <NavTabs
            value={current.value}
            tabs={DECISION_VIEWS.map(v => {
              const n = v.states.length ? v.states.reduce((sum, st) => sum + (counts[st] ?? 0), 0) : Object.values(counts).reduce((a, b) => a + b, 0);
              return {value: v.value, label: `${v.label} · ${n}`, href: `/bo/bot/decisions?view=${v.value}`};
            })}
          />
        </VStack>
      }>
      {error ? <ErrorState error={error} title="The decisions could not be loaded." /> : null}
      {!error && items.length === 0 ? (
        <Text type="supporting" color="secondary">
          {current.value === 'open' ? 'Nothing waits for a decision.' : 'No decisions here.'}
        </Text>
      ) : null}
      <VStack gap={3}>
        {items.map(d => (
          <DecisionCard key={d.id} decision={d} />
        ))}
      </VStack>
      <Pager base="/bo/bot/decisions" params={params} total={total} shown={items.length} nextCursor={nextCursor} noun="decisions" />
    </PageFrame>
  );
}

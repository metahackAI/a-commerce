'use client';

// Pieces the Ailism Bot pages share (docs/api/platform.md#bot): the tabs between its pages, the
// decision card with a person's buttons (ask again, feedback), the feedback form and list, and
// "Run now". Every person action goes through PersonButton; the bot itself never approves.

import {useState} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, StackItem, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {Button} from '@astryxdesign/core/Button';
import {TextArea} from '@astryxdesign/core/TextArea';
import {SegmentedControl, SegmentedControlItem} from '@astryxdesign/core/SegmentedControl';
import {Token} from '@astryxdesign/core/Token';
import {List, ListItem} from '@astryxdesign/core/List';

import {PersonButton} from '@/components/PersonButton';
import {dateTime, words} from '@/app/bo/_lib/format';
import {entityHref} from '@/app/bo/_lib/links';
import type {BotDecision, BotEffect, BotFeedback} from '@/app/bo/_lib/types';

import {useBoSession} from '../BoSession';
import {NavTabs} from '../NavTabs';
import {StateToken} from '../StateToken';
import {TargetCard, TargetRow} from '../targets';
import {useStoreEvents} from '../useStoreEvents';

export const AREA_LABEL: Record<string, string> = {
  pricing: 'Pricing',
  inventory: 'Inventory',
  promotions: 'Promotions',
  merchandising: 'Merchandising',
  catalog_content: 'Product pages',
  returns: 'Returns',
  questions_reviews: 'Questions and reviews',
  community: 'Community',
};

export const STATE_LABEL: Record<string, string> = {
  drafting: 'Drafting',
  needs_you: 'Needs you',
  waiting: 'Waiting for approval',
  running: 'Running',
  live: 'Live',
  rejected: 'Rejected',
  failed: 'Failed',
  undone: 'Undone',
  dismissed: 'Put away',
};

/** The bot's pages as tabs, under each page's title. */
export function BotTabs({value}: {value: 'dashboard' | 'decisions' | 'history' | 'settings'}) {
  return (
    <NavTabs
      value={value}
      tabs={[
        {value: 'dashboard', label: 'Dashboard', href: '/bo/bot'},
        {value: 'decisions', label: 'AI decisions', href: '/bo/bot/decisions'},
        {value: 'history', label: 'History', href: '/bo/bot/history'},
        {value: 'settings', label: 'Settings', href: '/bo/bot/settings'},
      ]}
    />
  );
}

/** Reloads the page's reads when the bot, a task or a change set moves. */
export function useBotEvents() {
  const router = useRouter();
  useStoreEvents(['bot.cycle.updated', 'task.updated', 'change_set.updated'], () => router.refresh());
}

export function canUseBot(scopes: string[] | undefined): boolean {
  return scopes?.includes('human:bot') ?? false;
}

/** "Run now": opens a cycle; the reading and analysis go on while the page watches. */
export function RunNowButton({isOff}: {isOff: boolean}) {
  const router = useRouter();
  const session = useBoSession();
  if (!canUseBot(session?.scopes)) return null;
  return (
    <PersonButton
      action={{
        label: 'Run now',
        method: 'POST',
        path: '/v1/platform/bot/cycles',
        body: {},
        confirm: 'Run Ailism Bot now? It reads the store, analyses it, and opens what it proposes as tasks for you to decide. Nothing changes until you approve.',
      }}
      isDisabled={isOff}
      onDone={() => router.refresh()}
    />
  );
}

export function effectText(e: BotEffect | undefined): string | null {
  if (!e?.metric) return null;
  const way = e.direction === 'down' ? 'down' : 'up';
  return `${words(e.metric)} ${way}${e.horizon_days ? ` within ${e.horizon_days} day${e.horizon_days === 1 ? '' : 's'}` : ''}`;
}

const RISK_COLOR: Record<string, 'green' | 'orange' | 'red'> = {low: 'green', medium: 'orange', high: 'red'};

export function RiskToken({risk}: {risk: string}) {
  return <Token label={`${words(risk)} risk`} size="sm" color={RISK_COLOR[risk] ?? 'gray'} />;
}

/** Feedback, a directive, or (with `decisionId`) feedback on one decision. */
export function FeedbackForm({decisionId, compact}: {decisionId?: string; compact?: boolean}) {
  const router = useRouter();
  const session = useBoSession();
  const [kind, setKind] = useState<'feedback' | 'directive'>('feedback');
  const [text, setText] = useState('');
  if (!canUseBot(session?.scopes)) return null;
  const said = text.trim();
  return (
    <VStack gap={2}>
      {decisionId ? null : (
        <SegmentedControl label="Kind" size="sm" value={kind} onChange={v => setKind(v as 'feedback' | 'directive')}>
          <SegmentedControlItem value="feedback" label="Feedback" />
          <SegmentedControlItem value="directive" label="Directive" />
        </SegmentedControl>
      )}
      <TextArea
        label={decisionId ? 'Feedback on this decision' : kind === 'directive' ? 'A directive the bot always follows' : 'Feedback for the bot'}
        description={
          kind === 'directive' && !decisionId
            ? 'It goes into every analysis until you close it.'
            : 'The bot reads open feedback in every analysis, until you close it.'
        }
        value={text}
        onChange={setText}
        maxLength={1000}
        rows={compact ? 2 : 3}
      />
      <HStack gap={2}>
        <PersonButton
          action={{
            label: kind === 'directive' && !decisionId ? 'Add directive' : 'Send feedback',
            method: 'POST',
            path: '/v1/platform/bot/feedback',
            body: {kind: decisionId ? 'feedback' : kind, text: said, ...(decisionId ? {bot_decision_id: decisionId} : {})},
            confirm: `Tell Ailism Bot: “${said}”?`,
          }}
          size="sm"
          variant="secondary"
          isDisabled={said === ''}
          onDone={() => {
            setText('');
            router.refresh();
          }}
        />
      </HStack>
    </VStack>
  );
}

/** Open feedback and directives, each closable. */
export function FeedbackList({items}: {items: BotFeedback[]}) {
  const router = useRouter();
  const session = useBoSession();
  if (items.length === 0) {
    return (
      <Text type="supporting" color="secondary">
        No open feedback.
      </Text>
    );
  }
  return (
    <List hasDividers edge="flush">
      {items.map(f => (
        <TargetRow
          key={f.id}
          target={{kind: 'bot_feedback', label: `${words(f.kind)}: ${f.text}`, ids: {bot_feedback_id: f.id}, term: 'acom:BotFeedback'}}>
          <ListItem
            label={f.text}
            description={`${words(f.kind)} · ${f.by} · ${dateTime(f.at)}${f.decision_id ? ` · on ${f.decision_id}` : ''}`}
            endContent={
              f.state === 'open' && canUseBot(session?.scopes) ? (
                <PersonButton
                  action={{
                    label: 'Close',
                    method: 'POST',
                    path: `/v1/platform/bot/feedback/${encodeURIComponent(f.id)}/close`,
                    body: {},
                    confirm: 'Close it? The bot stops reading it in its analyses.',
                  }}
                  size="sm"
                  variant="secondary"
                  onDone={() => router.refresh()}
                />
              ) : (
                <StateToken state={f.state} />
              )
            }
          />
        </TargetRow>
      ))}
    </List>
  );
}

/** "Ask again": what to change; the waiting change set is rejected with it and drafted anew. */
function AskAgain({decision, onClose}: {decision: BotDecision; onClose: () => void}) {
  const router = useRouter();
  const [text, setText] = useState('');
  const said = text.trim();
  const waiting = (decision.state === 'waiting' || decision.state === 'drafting') && decision.change_set_id;
  return (
    <VStack gap={2}>
      <TextArea
        label="What should change?"
        description="The agent drafts the change again with this. It is kept as feedback for the bot too."
        value={text}
        onChange={setText}
        maxLength={1000}
        rows={2}
      />
      <HStack gap={2}>
        <PersonButton
          action={{
            label: 'Ask again',
            method: 'POST',
            path: `/v1/platform/bot/decisions/${encodeURIComponent(decision.id)}/rerequest`,
            body: {text: said},
            confirm: waiting
              ? `Change set #${decision.change_set_number ?? ''} is rejected with your note, and a new draft is made with it. Go ahead?`
              : 'A new draft is made with your note. Go ahead?',
          }}
          size="sm"
          isDisabled={said === ''}
          onDone={() => {
            onClose();
            router.refresh();
          }}
        />
        <Button label="Cancel" size="sm" variant="ghost" onClick={onClose} />
      </HStack>
    </VStack>
  );
}

/** One decision: what and why, where its change is, and what a person can do with it. */
export function DecisionCard({decision, compact}: {decision: BotDecision; compact?: boolean}) {
  const session = useBoSession();
  const [open, setOpen] = useState<'again' | 'feedback' | null>(null);
  const effect = effectText(decision.expected_effect);
  const csHref = entityHref(decision.change_set_id);
  const canAsk = canUseBot(session?.scopes) && !['running', 'live'].includes(decision.state);
  const feedback = Array.isArray(decision.feedback) ? decision.feedback : [];
  return (
    <TargetCard
      target={{
        kind: 'bot_decision',
        label: `${decision.title} · ${STATE_LABEL[decision.state] ?? words(decision.state)}`,
        ids: {bot_decision_id: decision.id, task_id: decision.task_id, ...(decision.change_set_id ? {change_set_id: decision.change_set_id} : {})},
        term: 'acom:BotDecision',
      }}
      heading={
        <>
          <HStack gap={2} vAlign="center" wrap="wrap">
            <Text weight="semibold">{decision.title}</Text>
            <StateToken state={decision.state} />
          </HStack>
          <HStack gap={2} vAlign="center" wrap="wrap">
            <Token label={AREA_LABEL[decision.area] ?? words(decision.area)} size="sm" />
            <RiskToken risk={decision.risk} />
            <Text type="supporting" color="secondary">
              {dateTime(decision.at)}
              {decision.revises ? ` · asked again from ${decision.revises}` : ''}
            </Text>
          </HStack>
        </>
      }>
      <VStack gap={2}>
        <Text>{decision.reason}</Text>
        {!compact && decision.evidence.length > 0 ? (
          <VStack gap={1}>
            {decision.evidence.map(e => (
              <Text key={e} type="supporting" color="secondary">
                · {e}
              </Text>
            ))}
          </VStack>
        ) : null}
        {effect ? (
          <Text type="supporting" color="secondary">
            Expected: {effect}
          </Text>
        ) : null}
        {!compact ? (
          <Text type="supporting" color="secondary">
            Asked of the agent: “{decision.request}”
          </Text>
        ) : null}
        {decision.summary ? (
          <Text type="supporting" color="secondary">
            {decision.summary}
          </Text>
        ) : null}
        {!compact && decision.about.length > 0 ? (
          <HStack gap={2} wrap="wrap">
            {decision.about.map(a => {
              const href = entityHref(a.id);
              return href ? (
                <Link key={a.id} href={href}>
                  {a.id}
                </Link>
              ) : (
                <Text key={a.id} type="supporting">
                  {a.id}
                </Text>
              );
            })}
          </HStack>
        ) : null}
        {feedback.length > 0 ? <FeedbackList items={feedback} /> : null}
        <HStack gap={2} wrap="wrap" vAlign="center">
          {csHref ? (
            <Button
              label={decision.state === 'waiting' ? `Review #${decision.change_set_number ?? ''}` : `Change set #${decision.change_set_number ?? ''}`}
              size="sm"
              variant={decision.state === 'waiting' ? 'primary' : 'secondary'}
              href={csHref}
            />
          ) : null}
          <Button label="Task" size="sm" variant="ghost" href={`/bo/tasks/${encodeURIComponent(decision.task_id)}`} />
          {canAsk ? <Button label="Ask again" size="sm" variant="ghost" onClick={() => setOpen(open === 'again' ? null : 'again')} /> : null}
          {canUseBot(session?.scopes) ? (
            <Button label="Feedback" size="sm" variant="ghost" onClick={() => setOpen(open === 'feedback' ? null : 'feedback')} />
          ) : null}
          {typeof decision.feedback === 'number' && decision.feedback > 0 ? (
            <StackItem>
              <Text type="supporting" color="secondary">
                {decision.feedback} feedback
              </Text>
            </StackItem>
          ) : null}
        </HStack>
        {open === 'again' ? <AskAgain decision={decision} onClose={() => setOpen(null)} /> : null}
        {open === 'feedback' ? <FeedbackForm decisionId={decision.id} compact /> : null}
      </VStack>
    </TargetCard>
  );
}

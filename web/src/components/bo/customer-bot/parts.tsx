'use client';

// Pieces the Ailism Customer Bot pages share (docs/api/platform.md#customer_bot): the tabs between
// its pages, "Run now", the line that says what a session or run did, the session table, the
// persona's facts, and the page refresh while shoppers are at work.

import {useEffect} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';

import {PersonButton} from '@/components/PersonButton';
import {count, dateTime, money, words} from '@/app/bo/_lib/format';
import type {CustomerBotOutcome, CustomerBotSession, CustomerBotSummary, ShopperPersonaBody} from '@/app/bo/_lib/types';

import {useBoSession} from '../BoSession';
import {NavTabs} from '../NavTabs';
import {StateToken} from '../StateToken';
import {DataTable} from '../tables';
import {useStoreEvents} from '../useStoreEvents';

/** The bot's pages as tabs, under each page's title. */
export function CustomerBotTabs({value}: {value: 'dashboard' | 'sessions' | 'personas' | 'settings'}) {
  return (
    <NavTabs
      value={value}
      tabs={[
        {value: 'dashboard', label: 'Dashboard', href: '/bo/customer-bot'},
        {value: 'sessions', label: 'Sessions', href: '/bo/customer-bot/sessions'},
        {value: 'personas', label: 'Personas', href: '/bo/customer-bot/personas'},
        {value: 'settings', label: 'Settings', href: '/bo/customer-bot/settings'},
      ]}
    />
  );
}

/**
 * Reloads the page's reads when a run moves, and every few seconds while shoppers are at work
 * (their steps are recorded as they happen, with no event of their own).
 */
export function useCustomerBotRefresh(isWorking: boolean, seconds = 6) {
  const router = useRouter();
  useStoreEvents(['customer_bot.run.updated'], () => router.refresh());
  useEffect(() => {
    if (!isWorking) return;
    const t = window.setInterval(() => router.refresh(), seconds * 1000);
    return () => window.clearInterval(t);
  }, [isWorking, router, seconds]);
}

export function canUseCustomerBot(scopes: string[] | undefined): boolean {
  return scopes?.includes('human:bot') ?? false;
}

/** "Run now": queues a run; the gateway's workers take it within half a minute. */
export function RunNowButton() {
  const router = useRouter();
  const session = useBoSession();
  if (!canUseCustomerBot(session?.scopes)) return null;
  return (
    <PersonButton
      action={{
        label: 'Run now',
        method: 'POST',
        path: '/v1/platform/customer-bot/runs',
        body: {},
        confirm:
          'Send the shoppers in now? Each one signs in as a member (or signs up), shops on the storefront through the assistant, and may place real orders and write reviews in this demo store.',
      }}
      onDone={() => router.refresh()}
    />
  );
}

/** A money value a session wrote, which may be a number or text. */
export function spent(v: CustomerBotOutcome['spent'] | CustomerBotSummary['spent']): string {
  if (!v) return '—';
  const value = typeof v.value === 'string' ? Number(v.value) : v.value;
  return typeof value === 'number' && Number.isFinite(value) ? money({value, currency: v.currency || 'USD'}) : '—';
}

function plural(n: number, one: string, many = `${one}s`): string {
  return `${count(n)} ${n === 1 ? one : many}`;
}

/** What a session or run did, in one line: "5 chats · 1 order ($95.40) · 1 review". */
export function didLine(o: CustomerBotOutcome | CustomerBotSummary): string {
  const parts: string[] = [];
  if (o.chat_turns) parts.push(plural(o.chat_turns, 'chat'));
  if (o.cart_adds) parts.push(`${plural(o.cart_adds, 'cart add')}`);
  if (o.orders) parts.push(`${plural(o.orders, 'order')} (${spent(o.spent)})`);
  if (o.reviews) parts.push(plural(o.reviews, 'review'));
  if (o.memories) parts.push(plural(o.memories, 'memory', 'memories'));
  return parts.length > 0 ? parts.join(' · ') : 'Looked around';
}

export const ACCOUNT_LABEL: Record<string, string> = {existing: 'Member', created: 'Signed up'};

/** A member link that opens the customer page. */
export function MemberLink({id}: {id: string | null}) {
  if (!id) return <Text color="secondary">Signing up…</Text>;
  return <Link href={`/bo/customers/${encodeURIComponent(id)}`}>{id}</Link>;
}

/** Sessions as a table: who, as which persona, what for, and what they did. */
export function SessionTable({sessions, emptyTitle = 'No sessions yet'}: {sessions: CustomerBotSession[]; emptyTitle?: string}) {
  return (
    <DataTable
      rows={sessions}
      rowKey={s => s.id}
      target={s => ({
        kind: 'customer_bot_session',
        label: `${s.customer_id ?? 'new member'} · ${s.state} · ${didLine(s.outcome)}`,
        ids: {customer_bot_session_id: s.id, ...(s.customer_id ? {customer_id: s.customer_id} : {})},
        term: 'acom:CustomerBotSession',
      })}
      empty={{title: emptyTitle, description: 'Sessions appear as soon as the shoppers start.'}}
      columns={[
        {header: 'Started', width: '14%', cell: s => <Link href={`/bo/customer-bot/sessions/${encodeURIComponent(s.id)}`}>{dateTime(s.started)}</Link>},
        {
          header: 'Shopper',
          width: '30%',
          cell: s => (
            <VStack gap={0.5}>
              <HStack gap={2} vAlign="center" wrap="wrap">
                <MemberLink id={s.customer_id} />
                <Token label={ACCOUNT_LABEL[s.account] ?? words(s.account)} size="sm" />
                {s.persona_built ? <Token label="New persona" size="sm" color="blue" /> : null}
              </HStack>
              {s.persona ? (
                <Text type="supporting" color="secondary" maxLines={2}>
                  {s.persona}
                </Text>
              ) : null}
            </VStack>
          ),
        },
        {
          header: 'Came for',
          width: '26%',
          cell: s => (
            <Text type="supporting" maxLines={2}>
              {s.goal ?? '—'}
            </Text>
          ),
        },
        {header: 'State', cell: s => <StateToken state={s.state} />},
        {header: 'Did', cell: s => <Text type="supporting">{s.state === 'running' ? `${count(s.steps)} steps so far` : didLine(s.outcome)}</Text>},
      ]}
    />
  );
}

/** A persona's facts as a short list. */
export function PersonaFacts({p}: {p: ShopperPersonaBody}) {
  const list = (v: unknown) => (Array.isArray(v) ? v.filter(x => typeof x === 'string').join(', ') : '');
  const sizes = p.sizes ? [p.sizes.top && `top ${p.sizes.top}`, p.sizes.bottom && `bottom ${p.sizes.bottom}`, p.sizes.shoes && `shoes ${p.sizes.shoes}`].filter(Boolean).join(' · ') : '';
  const rows: [string, string][] = [
    ['Age', p.age_range ?? ''],
    ['Lives in', p.location ?? ''],
    ['Works as', p.occupation ?? ''],
    ['Style', list(p.style)],
    ['Colors', list(p.colors)],
    ['Sizes', sizes],
    ['Budget', typeof p.budget_per_order === 'number' ? `about $${p.budget_per_order} an order` : ''],
    ['Buys', list(p.categories)],
    ['Shops', [p.shopping_style && words(p.shopping_style), p.chattiness && `${words(p.chattiness)} talker`].filter(Boolean).join(', ')],
    ['Writes', p.voice ?? ''],
    ['Into', list(p.interests)],
  ];
  return (
    <MetadataList>
      {rows
        .filter(([, v]) => v)
        .map(([k, v]) => (
          <MetadataListItem key={k} label={k}>
            {v}
          </MetadataListItem>
        ))}
    </MetadataList>
  );
}

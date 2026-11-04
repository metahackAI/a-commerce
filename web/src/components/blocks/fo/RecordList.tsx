'use client';

// record_list: what a read returned that is not a list of products (chat.md §3.3) — the
// shopper's orders, subscriptions, returns, coupons and points, a product's reviews and
// questions, notices, sales running now, alerts, a delivery, a policy. Rows keep the tool's own
// values; each row is a target the shopper can point at or ask about.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {List, ListItem} from '@astryxdesign/core/List';
import {Text} from '@astryxdesign/core/Text';
import {Thumbnail} from '@astryxdesign/core/Thumbnail';

import type {UiBlock} from '@/ai/types';
import {useAiTarget} from '@/ai/useAiTarget';
import {dateTime, day, imageUrl, words} from '@/app/(storefront)/_lib/format';
import type {ImageRef} from '@/app/(storefront)/_lib/types';
import {AskButton, Intents} from '@/components/fo/Ask';
import {OpenInNewTab, useCanvasLink} from '@/components/fo/CanvasPages';
import {StatusTag, type Tone} from '@/components/fo/StatusTag';

import type {BlockProps} from '../types';
import {BlockFrame, dataOf, intentsFor, list} from './common';
import type {RecordListData} from './data';
import {moneyText} from './values';

type Row = RecordListData['rows'][number];

/** What each kind of row is, for the assistant's targets. */
const TERMS: Record<string, {id: string; term?: string}> = {
  orders: {id: 'order_id', term: 'schema:Order'},
  subscriptions: {id: 'subscription_id'},
  returns: {id: 'claim_id'},
  coupons: {id: 'coupon_id'},
  reviews: {id: 'review_id', term: 'schema:Review'},
  questions: {id: 'question_id', term: 'schema:Question'},
  notices: {id: 'notice_id', term: 'schema:Article'},
  faqs: {id: 'faq_id', term: 'schema:Question'},
  promotions: {id: 'promotion_id', term: 'schema:Offer'},
  collections: {id: 'collection_page_id', term: 'schema:Collection'},
  alerts: {id: 'subscription_id'},
  addresses: {id: 'address_id', term: 'schema:PostalAddress'},
  memories: {id: 'memory_id'},
};

function tone(status: string): Tone {
  if (['delivered', 'active', 'running', 'back_in_stock', 'approved', 'completed', 'refunded', 'primary', 'available', 'payment_complete'].includes(status)) return 'success';
  if (['problem', 'payment_due', 'waiting', 'requested', 'pending', 'paused', 'expiring'].includes(status)) return 'warning';
  if (['rejected', 'failed', 'cancelled', 'expired', 'used', 'returned', 'withdrawn'].includes(status)) return 'neutral';
  return 'info';
}

/** A store time as people read it: with the hour when it has one. */
function when(at: string): string {
  return at.includes('T') ? dateTime(at) : day(at);
}

function RecordRow({block, kind, row}: {block: UiBlock; kind: string; row: Row}) {
  const ids = TERMS[kind] ?? {id: 'record_id'};
  const target = useAiTarget({
    kind: 'record',
    label: [row.title, row.subtitle].filter(Boolean).join(' · '),
    ids: {[ids.id]: row.id},
    term: ids.term,
    blockId: block.block_id,
  });
  const amount = moneyText(row.amount);
  const dates = list(row.dates)
    .map(d => (d.label ? `${d.label} ${when(d.at)}` : when(d.at)))
    .join(' · ');
  const meta = list(row.meta).join(' · ');
  const link = useCanvasLink(row.href ?? '', row.title);
  const title = (
    <Text type="body" weight="semibold">
      {row.title}
    </Text>
  );
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      align="start"
      startContent={row.image ? <Thumbnail src={imageUrl(row.image as ImageRef) ?? undefined} alt={row.title} label={row.title} /> : undefined}
      label={
        <HStack gap={2} vAlign="center" wrap="wrap">
          {row.href ? <Link {...link}>{title}</Link> : title}
          {row.status ? <StatusTag label={words(row.status)} tone={tone(row.status)} /> : null}
          {typeof row.rating === 'number' ? (
            <Text type="supporting" weight="semibold">
              {row.rating} out of 5
            </Text>
          ) : null}
        </HStack>
      }
      description={
        <VStack gap={1.5}>
          {row.subtitle ? (
            <Text type="supporting" color="secondary">
              {row.subtitle}
            </Text>
          ) : null}
          {row.body ? <Text type="body">{row.body}</Text> : null}
          {amount || dates ? (
            <HStack gap={2} wrap="wrap" vAlign="center">
              {amount ? (
                <Text type="body" weight="semibold">
                  {amount}
                </Text>
              ) : null}
              {dates ? (
                <Text type="supporting" color="secondary">
                  {dates}
                </Text>
              ) : null}
            </HStack>
          ) : null}
          {meta ? (
            <Text type="supporting" color="secondary">
              {meta}
            </Text>
          ) : null}
          <Intents says={intentsFor(block, 'record', row.id)} about={target.context} />
        </VStack>
      }
      endContent={
        <HStack gap={1} vAlign="center">
          <AskButton target={target} label={row.title} />
          {row.href ? <OpenInNewTab href={row.href} /> : null}
        </HStack>
      }
    />
  );
}

function Summary({summary}: {summary: NonNullable<RecordListData['summary']>}) {
  const amount = moneyText(summary.amount);
  const value = amount || summary.value || (typeof summary.rating === 'number' ? `${summary.rating} out of 5` : '');
  const link = useCanvasLink(summary.href ?? '', summary.label);
  const head = (
    <HStack gap={2} vAlign="center" wrap="wrap">
      {value ? (
        <Text type="large" weight="semibold">
          {value}
        </Text>
      ) : null}
      <Text type="body" color="secondary">
        {summary.label}
      </Text>
    </HStack>
  );
  return (
    <VStack gap={1}>
      {summary.href ? <Link {...link}>{head}</Link> : head}
      {list(summary.detail).length > 0 ? (
        <Text type="supporting" color="secondary">
          {list(summary.detail).join(' · ')}
        </Text>
      ) : null}
    </VStack>
  );
}

export function RecordListBlock({block}: BlockProps) {
  const data = dataOf<RecordListData>(block);
  const kind = data.kind ?? 'records';
  const rows = list(data.rows);
  return (
    <BlockFrame block={block}>
      {data.summary ? <Summary summary={data.summary} /> : null}
      {rows.length > 0 ? (
        <List hasDividers edge="flush">
          {rows.map(row => (
            <RecordRow key={row.id} block={block} kind={kind} row={row} />
          ))}
        </List>
      ) : !data.summary ? (
        <Text type="body" color="secondary">
          Nothing here yet.
        </Text>
      ) : null}
    </BlockFrame>
  );
}

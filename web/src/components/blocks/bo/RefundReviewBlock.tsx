'use client';

// `refund_review` block (docs/api/chat.md §3.3): a batch of return requests judged against the
// policy. The summary, each request with the suggestion and why, the checks, the messages that
// would go out, and the change set that holds it all. Approving is the block's person button.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {MetadataList} from '@astryxdesign/core/MetadataList';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';

import type {RefundReviewBlockData} from '@/app/bo/_lib/blocks';
import {count, money, value, words} from '@/app/bo/_lib/format';
import {ChecksList} from '@/components/bo/ChangeParts';
import {StateToken} from '@/components/bo/StateToken';
import {AskHeaderCell, TargetBlock, TargetFact, TargetRow} from '@/components/bo/targets';

import type {BlockProps} from '../types';
import {BlockFrame, intentsLookup} from './parts';

export function RefundReviewBlock({block}: BlockProps) {
  const d = block.data as RefundReviewBlockData;
  const intentsFor = intentsLookup(block);
  const ids: Record<string, string> = d.change_set_id ? {change_set_id: d.change_set_id} : {};
  const s = d.summary;

  return (
    <BlockFrame block={block} kind="refund_review" id={d.change_set_id ?? block.block_id} label="Return requests">
      {s ? (
        <MetadataList columns="multi">
          {s.waiting !== undefined ? (
            <TargetFact label="Waiting" target={{kind: 'summary', label: `${s.waiting} waiting`, ids: {summary: 'waiting', ...ids}, blockId: block.block_id}}>
              <Text weight="medium">{count(s.waiting)}</Text>
            </TargetFact>
          ) : null}
          {s.refund ? (
            <TargetFact
              label="Refund"
              target={{kind: 'summary', label: `Refund ${s.refund.count ?? ''} for ${money(s.refund.amount)}`, ids: {summary: 'refund', ...ids}, blockId: block.block_id}}>
              <Text weight="medium">{`${count(s.refund.count)} · ${money(s.refund.amount)}`}</Text>
            </TargetFact>
          ) : null}
          {s.ask_photos !== undefined ? (
            <TargetFact label="Ask for photos" target={{kind: 'summary', label: `${s.ask_photos} photo requests`, ids: {summary: 'ask_photos', ...ids}, blockId: block.block_id}}>
              <Text weight="medium">{count(s.ask_photos)}</Text>
            </TargetFact>
          ) : null}
          {s.decline !== undefined ? (
            <TargetFact label="Decline" target={{kind: 'summary', label: `${s.decline} to decline`, ids: {summary: 'decline', ...ids}, blockId: block.block_id}}>
              <Text weight="medium">{count(s.decline)}</Text>
            </TargetFact>
          ) : null}
        </MetadataList>
      ) : null}

      {d.requests && d.requests.length > 0 ? (
        <Table density="compact" verticalAlign="top">
          <TableHeader>
            <TableRow isHeaderRow>
              <TableHeaderCell>Order</TableHeaderCell>
              <TableHeaderCell>Item</TableHeaderCell>
              <TableHeaderCell>Amount</TableHeaderCell>
              <TableHeaderCell>Reason</TableHeaderCell>
              <TableHeaderCell>Suggested</TableHeaderCell>
              <AskHeaderCell />
            </TableRow>
          </TableHeader>
          <TableBody>
            {d.requests.map(r => {
              const id = r.claim_id ?? r.order_number;
              return (
                <TargetRow
                  key={id}
                  target={{
                    kind: 'request_row',
                    label: `#${r.order_number} ${r.item} ${money(r.amount)} · ${r.suggested ?? ''}${r.because ? ` because ${r.because}` : ''}`,
                    ids: {...(r.claim_id ? {claim_id: r.claim_id} : {order_number: r.order_number}), ...ids},
                    term: 'acom:Claim',
                    blockId: block.block_id,
                  }}
                  intents={intentsFor('request_row', id)}>
                  <TableCell>
                    {r.claim_id ? <Link href={`/bo/claims/${encodeURIComponent(r.claim_id)}`}>{`#${r.order_number}`}</Link> : <Text>{`#${r.order_number}`}</Text>}
                  </TableCell>
                  <TableCell>
                    <Text>{r.item}</Text>
                  </TableCell>
                  <TableCell>
                    <Text hasTabularNumbers>{money(r.amount)}</Text>
                  </TableCell>
                  <TableCell>
                    <VStack gap={0}>
                      <Text>{words(r.reason)}</Text>
                      <Text type="supporting" color="secondary">
                        {[
                          r.evidence !== undefined ? value(r.evidence) : null,
                          r.day !== undefined && r.day !== null ? `${r.day} ${r.day === 1 ? 'day' : 'days'} since delivery` : null,
                        ]
                          .filter(Boolean)
                          .join(' · ')}
                      </Text>
                    </VStack>
                  </TableCell>
                  <TableCell>
                    <VStack gap={0.5}>
                      <StateToken state={r.suggested} />
                      {r.because ? (
                        <Text type="supporting" color="secondary">
                          {r.because}
                        </Text>
                      ) : null}
                      {(r.flags ?? []).map(f => (
                        <Text key={f.rule ?? f.detail} type="supporting" weight="medium">
                          {f.detail ?? words(f.rule)}
                        </Text>
                      ))}
                    </VStack>
                  </TableCell>
                </TargetRow>
              );
            })}
          </TableBody>
        </Table>
      ) : null}

      {d.checks && d.checks.length > 0 ? (
        <VStack gap={2}>
          <Heading level={4}>Checks</Heading>
          <ChecksList checks={d.checks} ids={ids} blockId={block.block_id} intentsFor={intentsFor} />
        </VStack>
      ) : null}

      {d.messages && d.messages.length > 0 ? (
        <VStack gap={2}>
          <Heading level={4}>Messages to customers</Heading>
          {d.messages.map((m, i) => {
            const text = typeof m === 'string' ? m : (m.text ?? value(m));
            const to = typeof m === 'string' ? null : [m.order_number ? `#${m.order_number}` : null, m.to, m.kind ? words(m.kind) : null].filter(Boolean).join(' · ');
            return (
              <TargetBlock key={i} target={{kind: 'message', label: `${to ? `${to}: ` : ''}${text}`, ids: {message: String(i), ...ids}, blockId: block.block_id}}>
                {to ? (
                  <Text type="supporting" color="secondary">
                    {to}
                  </Text>
                ) : null}
                <Text>{text}</Text>
              </TargetBlock>
            );
          })}
        </VStack>
      ) : null}

      {d.change_set_id ? (
        <HStack gap={2}>
          <Link href={`/bo/change-sets/${encodeURIComponent(d.change_set_id)}`}>Open the change set</Link>
        </HStack>
      ) : null}
    </BlockFrame>
  );
}

'use client';

// `sales_explainer` block (docs/api/chat.md §3.3): a sales question answered, read only. The
// headline figures, the daily revenue around the day, the causes with their evidence and
// estimated effect (and what stays unexplained), and the next steps the agent can take. Each
// next step is a `say` action whose words are its request; it becomes a change set for another
// agent, never a change made here. Every figure is the block's own, formatted only.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {MetadataList} from '@astryxdesign/core/MetadataList';
import {Button} from '@astryxdesign/core/Button';
import {Token} from '@astryxdesign/core/Token';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';

import {useAi} from '@/ai/AiProvider';
import type {BlockAction} from '@/ai/types';
import type {SalesExplainerBlockData} from '@/app/bo/_lib/blocks';
import {count, isMoney, money, percent, value} from '@/app/bo/_lib/format';
import {useAgentDock} from '@/components/bo/AgentDock';
import {SalesChart} from '@/components/bo/SalesChart';
import {TargetFact, TargetRow} from '@/components/bo/targets';

import type {BlockProps} from '../types';
import {BlockFrame, intentsLookup} from './parts';

type Say = Extract<BlockAction, {kind: 'say'}>;

const show = (v: unknown) => (isMoney(v) ? money(v) : value(v));

/** A next step: a `say` action that carries the sentence it sends. */
function isNextStep(a: BlockAction): a is Say {
  return a.kind === 'say' && typeof (a.args as Record<string, unknown> | undefined)?.request === 'string';
}

export function SalesExplainerBlock({block}: BlockProps) {
  const d = block.data as SalesExplainerBlockData;
  const ai = useAi();
  const dock = useAgentDock();
  const intentsFor = intentsLookup(block);
  const s = d.summary ?? {};
  const orders = s.orders && typeof s.orders === 'object' ? s.orders : {count: typeof s.orders === 'number' ? s.orders : null, baseline: null};
  const steps = block.actions.filter(isNextStep);
  const fig = (id: string, label: string) => ({kind: 'figure', label, ids: {figure: id}, blockId: block.block_id, term: 'schema:Observation'});

  return (
    <BlockFrame
      block={block}
      kind="sales_explainer"
      id={block.block_id}
      label="Sales explained"
      meta={
        <>
          <Token label="Read only" size="sm" />
          <Text type="supporting" color="secondary">
            Nothing changed
          </Text>
        </>
      }
      hideActions={isNextStep}>
      <MetadataList columns="multi">
        <TargetFact label={`Revenue${d.period?.label ? `, ${d.period.label}` : ''}`} target={fig('revenue', `Revenue: ${money(s.revenue)}`)}>
          <VStack gap={0}>
            <Text weight="semibold" hasTabularNumbers>
              {money(s.revenue)}
            </Text>
            {s.baseline ? (
              <Text type="supporting" color="secondary" hasTabularNumbers>
                {s.baseline_label ?? 'Basis'} {money(s.baseline)}
              </Text>
            ) : null}
          </VStack>
        </TargetFact>
        <TargetFact label="Difference" target={fig('difference', `Difference: ${money(s.difference)} (${percent(s.change_rate)})`)}>
          <VStack gap={0}>
            <Text weight="semibold" hasTabularNumbers>
              {money(s.difference)} ({percent(s.change_rate)})
            </Text>
            {s.basis_label ? (
              <Text type="supporting" color="secondary">
                {s.basis_label}
              </Text>
            ) : null}
          </VStack>
        </TargetFact>
        <TargetFact label="Orders" target={fig('orders', `Orders: ${count(orders.count ?? null)}`)}>
          <VStack gap={0}>
            <Text weight="semibold" hasTabularNumbers>
              {count(orders.count ?? null)}
            </Text>
            {typeof orders.baseline === 'number' ? (
              <Text type="supporting" color="secondary" hasTabularNumbers>
                average {count(orders.baseline)}
              </Text>
            ) : null}
          </VStack>
        </TargetFact>
      </MetadataList>

      {d.series && d.series.length > 0 ? (
        <SalesChart
          points={d.series}
          baseline={s.baseline ?? null}
          baselineLabel={s.baseline_label}
          marked={d.period?.valid_through}
          blockId={block.block_id}
          title={`Daily revenue, last ${d.series.length} days`}
        />
      ) : null}

      {d.causes && d.causes.length > 0 ? (
        <VStack gap={2}>
          <HStack gap={2} vAlign="end" hAlign="between" wrap="wrap">
            <Heading level={4}>What explains it</Heading>
            {s.basis_label ? (
              <Text type="supporting" color="secondary">
                Estimates {s.basis_label.replace(/^vs /, 'from ')}, not exact
              </Text>
            ) : null}
          </HStack>
          <Table density="compact" verticalAlign="top">
            <TableHeader>
              <TableRow isHeaderRow>
                <TableHeaderCell>Cause</TableHeaderCell>
                <TableHeaderCell>Evidence</TableHeaderCell>
                <TableHeaderCell>Estimated</TableHeaderCell>
                <TableHeaderCell />
              </TableRow>
            </TableHeader>
            <TableBody>
              {d.causes.map(c => (
                <TargetRow
                  key={c.cause}
                  target={{
                    kind: 'cause',
                    label: `${c.title ?? c.cause}: ${show(c.estimated)}`,
                    ids: {cause: c.cause, ...(c.ids && c.ids.length > 0 ? {evidence: c.ids.join(',')} : {})},
                    blockId: block.block_id,
                  }}
                  intents={intentsFor('cause', c.cause)}>
                  <TableCell>
                    <VStack gap={0}>
                      <Text weight="semibold">{c.title ?? c.cause}</Text>
                      {c.detail ? (
                        <Text type="supporting" color="secondary">
                          {c.detail}
                        </Text>
                      ) : null}
                    </VStack>
                  </TableCell>
                  <TableCell>
                    <Text color={c.cause === 'unexplained' ? 'secondary' : undefined}>{c.evidence ?? '—'}</Text>
                  </TableCell>
                  <TableCell>
                    <HStack hAlign="end">
                      <Text weight="semibold" hasTabularNumbers>
                        {show(c.estimated)}
                      </Text>
                    </HStack>
                  </TableCell>
                </TargetRow>
              ))}
            </TableBody>
          </Table>
        </VStack>
      ) : null}

      {steps.length > 0 ? (
        <VStack gap={2}>
          <HStack gap={2} vAlign="end" hAlign="between" wrap="wrap">
            <Heading level={4}>What I can do next</Heading>
            <Text type="supporting" color="secondary">
              Each becomes a change set for you to approve
            </Text>
          </HStack>
          <HStack gap={2} wrap="wrap">
            {steps.map(a => (
              <Button
                key={a.action_id}
                label={a.label}
                onClick={() => {
                  dock.open();
                  void ai.sendEvent(block.block_id, a.action_id, a.label, a.args as Record<string, unknown>);
                }}
              />
            ))}
          </HStack>
        </VStack>
      ) : null}
    </BlockFrame>
  );
}

'use client';

// `sales_explainer` block (docs/api/chat.md §3.3): a sales question answered, read only. The
// headline figures, the daily series, the causes with their evidence and estimated effect, and
// the next steps the agent can take (each a `say` that becomes a change set request).

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {MetadataList} from '@astryxdesign/core/MetadataList';
import {Button} from '@astryxdesign/core/Button';

import {useAi} from '@/ai/AiProvider';
import type {SalesExplainerBlockData} from '@/app/bo/_lib/blocks';
import {date, isMoney, money, value, words} from '@/app/bo/_lib/format';
import {useAgentDock} from '@/components/bo/AgentDock';
import {TargetFact} from '@/components/bo/targets';

import type {BlockProps} from '../types';
import {BlockFrame, intentsLookup} from './parts';
import {RowsTable} from './rows';

const show = (v: unknown) => (isMoney(v) ? money(v) : value(v));

export function SalesExplainerBlock({block}: BlockProps) {
  const d = block.data as SalesExplainerBlockData;
  const ai = useAi();
  const dock = useAgentDock();
  const intentsFor = intentsLookup(block);
  const summary = Object.entries(d.summary ?? {});

  return (
    <BlockFrame block={block} kind="sales_explainer" id={block.block_id} label="Sales explained">
      {summary.length > 0 ? (
        <MetadataList columns="multi">
          {summary.map(([k, v]) => (
            <TargetFact key={k} label={words(k)} target={{kind: 'figure', label: `${words(k)}: ${show(v)}`, ids: {figure: k}, blockId: block.block_id}}>
              <Text weight="semibold" hasTabularNumbers>
                {show(v)}
              </Text>
            </TargetFact>
          ))}
        </MetadataList>
      ) : null}

      {d.causes && d.causes.length > 0 ? (
        <VStack gap={2}>
          <Heading level={4}>Why</Heading>
          <RowsTable
            rows={d.causes.map(c => ({id: c.cause, name: c.cause, evidence: c.evidence ?? null, estimated: show(c.estimated)}))}
            columns={[
              {key: 'evidence', label: 'Evidence'},
              {key: 'estimated', label: 'Estimated effect'},
            ]}
            kind="cause"
            blockId={block.block_id}
            intentsFor={intentsFor}
          />
        </VStack>
      ) : null}

      {d.series && d.series.length > 0 ? (
        <VStack gap={2}>
          <Heading level={4}>By day</Heading>
          <RowsTable
            rows={d.series.map(s => ({id: s.date, name: date(s.date), revenue: show(s.revenue)}))}
            columns={[{key: 'revenue', label: 'Revenue'}]}
            kind="series_point"
            blockId={block.block_id}
          />
        </VStack>
      ) : null}

      {d.next && d.next.length > 0 ? (
        <VStack gap={2}>
          <Heading level={4}>What I can do next</Heading>
          <HStack gap={2} wrap="wrap">
            {d.next.map(n => (
              <Button
                key={n.label}
                label={n.label}
                onClick={() => {
                  dock.open();
                  void ai.send(n.request);
                }}
              />
            ))}
          </HStack>
        </VStack>
      ) : null}
    </BlockFrame>
  );
}

'use client';

// `product_drafts` block (docs/api/chat.md §3.3): product drafts being written from a supplier
// sheet. Progress as the agent reports it, each draft as a row people can ask about, the house
// rules it followed, and the checks.

import {VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {ProgressBar} from '@astryxdesign/core/ProgressBar';
import {List, ListItem} from '@astryxdesign/core/List';

import type {ProductDraftsBlockData} from '@/app/bo/_lib/blocks';
import {count, money, value} from '@/app/bo/_lib/format';
import {ChecksList} from '@/components/bo/ChangeParts';

import type {BlockProps} from '../types';
import {BlockFrame, intentsLookup} from './parts';
import {RowsTable} from './rows';

function amount(v: unknown): string {
  if (typeof v === 'object' && v !== null && 'value' in v) return money(v as {value: number; currency: string});
  return value(v);
}

export function ProductDraftsBlock({block}: BlockProps) {
  const d = block.data as ProductDraftsBlockData;
  const intentsFor = intentsLookup(block);
  const p = d.progress;
  const rows = (d.drafts ?? []).map(x => ({
    draft_id: x.draft_id ?? x.source_code ?? x.name,
    name: x.name,
    source_code: x.source_code ?? null,
    cost: amount(x.cost),
    price: amount(x.price),
    sizes: x.sizes ?? null,
    status: x.status ?? null,
    note: x.note ?? null,
  }));

  return (
    <BlockFrame block={block} kind="product_drafts" id={block.block_id} label="Product drafts">
      {p && typeof p.total === 'number' && typeof p.ready === 'number' ? (
        <VStack gap={1}>
          <ProgressBar label="Drafts ready" value={p.ready} max={p.total} isLabelHidden variant="accent" />
          <Text type="supporting" color="secondary">
            {[`${count(p.ready)} of ${count(p.total)} ready`, p.needs_you ? `${count(p.needs_you)} need you` : null, p.working ? `${count(p.working)} in progress` : null]
              .filter(Boolean)
              .join(' · ')}
          </Text>
        </VStack>
      ) : null}
      <RowsTable
        rows={rows}
        columns={[
          {key: 'source_code', label: 'Supplier code'},
          {key: 'cost', label: 'Cost'},
          {key: 'price', label: 'Price'},
          {key: 'sizes', label: 'Sizes'},
          {key: 'status', label: 'Status'},
          {key: 'note', label: 'Note'},
        ]}
        kind="draft_row"
        blockId={block.block_id}
        intentsFor={intentsFor}
        empty="No drafts yet"
      />
      {d.rules && d.rules.length > 0 ? (
        <VStack gap={1}>
          <Heading level={4}>Rules followed</Heading>
          <List density="compact" listStyle="disc">
            {d.rules.map((r, i) => (
              <ListItem key={i} label={typeof r === 'string' ? r : (r.text ?? r.rule ?? '')} />
            ))}
          </List>
        </VStack>
      ) : null}
      {d.checks && d.checks.length > 0 ? (
        <VStack gap={2}>
          <Heading level={4}>Checks</Heading>
          <ChecksList checks={d.checks} blockId={block.block_id} intentsFor={intentsFor} />
        </VStack>
      ) : null}
    </BlockFrame>
  );
}

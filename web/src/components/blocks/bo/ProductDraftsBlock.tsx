'use client';

// `product_drafts` block (docs/api/chat.md §3.3): the drafts a supplier sheet becomes (scene B4),
// from the `batch_import` preview. Progress as commerce counted it, each draft as a row people can
// ask about (supplier code, cost → price from the pricing rule, sizes, what is still open), the
// rules the import used, and its checks: passed, failed, or open (drafts saved but waiting for a
// person). Saving is the block's person button; nothing is published here.

import {HStack, StackItem, VStack} from '@astryxdesign/core/Layout';
import {Heading, Text} from '@astryxdesign/core/Text';
import {ProgressBar} from '@astryxdesign/core/ProgressBar';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Icon} from '@astryxdesign/core/Icon';

import {useAiTarget} from '@/ai/useAiTarget';
import type {ProductDraftsBlockData} from '@/app/bo/_lib/blocks';
import {count, money, value, words} from '@/app/bo/_lib/format';
import {AskButton, SuggestedIntents} from '@/components/bo/targets';

import type {BlockProps} from '../types';
import {BlockFrame, intentsLookup} from './parts';
import {RowsTable} from './rows';

type DraftCheck = NonNullable<ProductDraftsBlockData['checks']>[number];

function amount(v: unknown): string {
  if (typeof v === 'object' && v !== null && 'value' in v) return money(v as {value: number; currency: string});
  return value(v);
}

function CheckLine({c, blockId, intents}: {c: DraftCheck; blockId: string; intents?: string[]}) {
  const status = c.status ?? (c.passed ? 'passed' : 'failed');
  const t = useAiTarget({
    kind: 'check_row',
    label: `${c.label ?? words(c.rule)} · ${status}${c.detail ? ` · ${c.detail}` : ''}`,
    ids: {rule: c.rule},
    term: 'acom:Check',
    blockId,
  });
  const icon = status === 'passed' ? 'success' : status === 'open' ? 'warning' : 'error';
  return (
    <HStack ref={t.ref} {...t.props} gap={3} vAlign="start" paddingBlock={1}>
      <Icon icon={icon} color={icon} size="sm" label={words(status)} />
      <StackItem size="fill">
        <VStack gap={0.5}>
          <HStack gap={2} vAlign="center" wrap="wrap">
            <Text weight="medium">{c.label ?? words(c.rule)}</Text>
            <Text type="supporting" color="secondary">
              {status === 'open' ? 'open' : c.rule}
            </Text>
          </HStack>
          {c.detail ? (
            <Text type="supporting" color="secondary">
              {c.detail}
            </Text>
          ) : null}
          <SuggestedIntents intents={intents} target={t} />
        </VStack>
      </StackItem>
      <AskButton target={t} label={c.label ?? words(c.rule)} />
    </HStack>
  );
}

export function ProductDraftsBlock({block}: BlockProps) {
  const d = block.data as ProductDraftsBlockData;
  const intentsFor = intentsLookup(block);
  const p = d.progress;
  const rows = (d.drafts ?? []).map(x => ({
    draft_id: x.draft_id ?? x.product_id ?? x.source_code ?? x.name,
    name: x.name,
    source_code: x.source_code ?? null,
    cost_price: `${amount(x.cost)} → ${amount(x.price)}`,
    sizes: Array.isArray(x.sizes) ? x.sizes.join(', ') : (x.sizes ?? null),
    status: x.status ? words(x.status) : null,
    note: x.note ?? (x.open_fields?.length ? `Open: ${x.open_fields.map(words).join(', ')}` : null),
  }));

  return (
    <BlockFrame block={block} kind="product_drafts" id={block.block_id} label="Product drafts">
      {d.file_name || d.change_set_id ? (
        <MetadataList columns="multi">
          {d.file_name ? <MetadataListItem label="Sheet">{`${d.file_name}${typeof d.rows === 'number' ? ` · ${count(d.rows)} rows` : ''}`}</MetadataListItem> : null}
          {d.provider_name ? <MetadataListItem label="Supplier">{d.provider_name}</MetadataListItem> : null}
          {d.change_set_id ? <MetadataListItem label="Change set">{`${d.change_set_id}${d.state ? ` · ${words(d.state)}` : ''}`}</MetadataListItem> : null}
        </MetadataList>
      ) : null}
      {p && typeof p.total === 'number' && typeof p.ready === 'number' ? (
        <VStack gap={1}>
          <ProgressBar label="Drafts ready" value={p.ready} max={p.total} isLabelHidden variant="accent" />
          <Text type="supporting" color="secondary">
            {[`${count(p.ready)} of ${count(p.total)} ready`, p.needs_you ? `${count(p.needs_you)} need you` : null, p.working ? `${count(p.working)} still working` : null]
              .filter(Boolean)
              .join(' · ')}
          </Text>
        </VStack>
      ) : null}
      <RowsTable
        rows={rows}
        columns={[
          {key: 'source_code', label: 'Supplier code'},
          {key: 'cost_price', label: 'Cost → price'},
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
          <Heading level={4}>Rules used</Heading>
          <MetadataList columns="multi">
            {d.rules.map((r, i) =>
              typeof r === 'string' ? (
                <MetadataListItem key={i} label="Rule">
                  {r}
                </MetadataListItem>
              ) : (
                <MetadataListItem key={i} label={r.label ?? words(r.rule ?? '')}>
                  {r.text ?? r.rule ?? ''}
                </MetadataListItem>
              ),
            )}
          </MetadataList>
        </VStack>
      ) : null}
      {d.checks && d.checks.length > 0 ? (
        <VStack gap={2}>
          <Heading level={4}>Checks</Heading>
          {d.checks.map(c => (
            <CheckLine key={c.rule} c={c} blockId={block.block_id} intents={intentsFor('check_row', c.rule)} />
          ))}
        </VStack>
      ) : null}
      {d.visibility ? (
        <Text type="supporting" color="secondary">
          {d.visibility}
        </Text>
      ) : null}
    </BlockFrame>
  );
}

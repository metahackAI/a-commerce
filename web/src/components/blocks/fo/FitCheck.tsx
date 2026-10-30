'use client';

// fit_check: which size, why, and what the answer rests on (chat.md §3.3, scene F3): the size
// options, measurements side by side, what reviews say, and what Northwind used and did not use.

import {Card} from '@astryxdesign/core/Card';
import {Grid} from '@astryxdesign/core/Grid';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Link} from '@astryxdesign/core/Link';
import {MetadataList, MetadataListItem} from '@astryxdesign/core/MetadataList';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';
import {Text} from '@astryxdesign/core/Text';
import {Thumbnail} from '@astryxdesign/core/Thumbnail';

import type {UiBlock} from '@/ai/types';
import {useAiTarget} from '@/ai/useAiTarget';
import {imageUrl} from '@/app/(storefront)/_lib/format';
import type {ImageRef} from '@/app/(storefront)/_lib/types';
import {Intents} from '@/components/fo/Ask';

import type {BlockProps} from '../types';
import {ActionButton, BlockActions, BlockFrame, dataOf, intentsFor, list} from './common';
import type {FitCheckData} from './data';
import {moneyText, textOf} from './values';

type Option = FitCheckData['options'][number];

/** The block's say button for one size, if it carries one (`args.size`, or a label naming it). */
function actionFor(block: UiBlock, size: string) {
  return block.actions.find(
    a => a.kind === 'say' && (a.args?.size === size || new RegExp(`\\b${size.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\b`).test(a.label)),
  );
}

function OptionCard({block, option, isFirst}: {block: UiBlock; option: Option; isFirst: boolean}) {
  const target = useAiTarget({kind: 'fit_option', label: `${option.label ?? 'Size'} ${option.size}`, ids: {size: option.size}, blockId: block.block_id});
  const action = actionFor(block, option.size);
  const priceText = moneyText(option.price);
  return (
    <Card ref={target.ref} {...target.props} variant={option.recommended || (option.recommended === undefined && isFirst) ? 'default' : 'muted'}>
      <VStack gap={1.5}>
        {option.label ? (
          <Text type="supporting" color="secondary">
            {option.label}
          </Text>
        ) : null}
        <Text type="display-3" weight="semibold">
          {option.size}
        </Text>
        {option.reason ? <Text type="body">{option.reason}</Text> : null}
        {action ? (
          <HStack>
            <ActionButton action={action} />
          </HStack>
        ) : priceText ? (
          <Text type="body" weight="semibold" hasTabularNumbers>
            {priceText}
          </Text>
        ) : null}
        <Intents says={intentsFor(block, 'fit_option', option.size)} />
      </VStack>
    </Card>
  );
}

function Measurements({block, m}: {block: UiBlock; m: FitCheckData['measurements']}) {
  const target = useAiTarget({kind: 'measurement_table', label: 'Measurements', ids: {measurements: block.block_id}, term: 'schema:QuantitativeValue', blockId: block.block_id});
  const columns = list(m.columns).map(textOf);
  const rows = list(m.rows as unknown[]).map(r => {
    if (Array.isArray(r)) return {label: textOf(r[0]), values: r.slice(1)};
    const o = (r ?? {}) as {label?: unknown; values?: unknown[]};
    return {label: textOf(o.label), values: list(o.values)};
  });
  const width = rows.reduce((n, r) => Math.max(n, r.values.length), 0);
  // Columns may name the label column too; keep the last `width` as value headers.
  const valueHeaders = columns.length > width ? columns.slice(columns.length - width) : columns;
  const labelHeader = columns.length > width ? columns[0] : '';
  return (
    <VStack gap={1.5} ref={target.ref} {...target.props}>
      <HStack gap={2} vAlign="center" wrap="wrap">
        <Text type="label" weight="semibold">
          Measurements
        </Text>
        {m.caption ? (
          <Text type="supporting" color="secondary">
            {m.caption}
          </Text>
        ) : null}
      </HStack>
      <Table density="compact">
        <TableHeader>
          <TableRow>
            <TableHeaderCell scope="col">{labelHeader}</TableHeaderCell>
            {valueHeaders.map(h => (
              <TableHeaderCell key={h} scope="col">
                {h}
              </TableHeaderCell>
            ))}
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map(r => (
            <TableRow key={r.label}>
              <TableCell scope="row">{r.label}</TableCell>
              {Array.from({length: width}, (_, i) => (
                <TableCell key={i}>
                  <Text type="body" hasTabularNumbers>
                    {textOf(r.values[i])}
                  </Text>
                </TableCell>
              ))}
            </TableRow>
          ))}
        </TableBody>
      </Table>
      <Intents says={intentsFor(block, 'measurement_table', block.block_id)} />
    </VStack>
  );
}

/** `{label|source|key, value}` or text, as label/value pairs for a MetadataList. */
function pairs(items: unknown[]): {label: string; value: string}[] {
  return items.map(i => {
    if (i && typeof i === 'object' && !Array.isArray(i)) {
      const o = i as Record<string, unknown>;
      const label = textOf(o.label ?? o.source ?? o.key ?? o.kind ?? '');
      const value = textOf(o.value ?? o.text ?? o.detail ?? '');
      return {label: label || value, value: label ? value : ''};
    }
    return {label: textOf(i), value: ''};
  });
}

function Facts({title, aside, items}: {title: string; aside?: string; items: {label: string; value: string}[]}) {
  if (items.length === 0) return null;
  return (
    <Card>
      <VStack gap={2}>
        <HStack gap={2} justify="between" vAlign="center" wrap="wrap">
          <Text type="label" weight="semibold">
            {title}
          </Text>
          {aside ? (
            <Text type="supporting" color="secondary">
              {aside}
            </Text>
          ) : null}
        </HStack>
        <MetadataList>
          {items.map((it, i) => (
            <MetadataListItem key={`${it.label}-${i}`} label={it.label}>
              {it.value}
            </MetadataListItem>
          ))}
        </MetadataList>
      </VStack>
    </Card>
  );
}

function UsedFact({block, label, value, used}: {block: UiBlock; label: string; value: string; used: boolean}) {
  const target = useAiTarget({kind: 'profile_fact', label: value ? `${label}: ${value}` : label, ids: {fact: label}, blockId: block.block_id});
  return (
    <MetadataListItem label={used ? label || 'Used' : 'Not used'}>
      <VStack gap={0.5} ref={target.ref} {...target.props}>
        <Text type="body">{used ? value || label : value ? `${label}: ${value}` : label}</Text>
      </VStack>
    </MetadataListItem>
  );
}

export function FitCheckBlock({block}: BlockProps) {
  const data = dataOf<FitCheckData>(block);
  const product = data.product ?? {};
  const options = list(data.options);
  const reviews = data.reviews ?? {};
  const used = pairs(list(data.used));
  const notUsed = pairs(list(data.not_used));
  const fit = list(reviews.fit).map(textOf).filter(Boolean);
  const mentions = list(reviews.mentions).map(m => {
    if (m && typeof m === 'object' && 'text' in (m as object)) {
      const o = m as {text?: unknown; count?: unknown};
      return typeof o.count === 'number' ? `“${textOf(o.text)}” ${o.count} times` : `“${textOf(o.text)}”`;
    }
    return textOf(m);
  });
  const reviewItems = [
    ...(fit.length > 0 ? [{label: 'Fit', value: fit.join(', ')}] : []),
    ...(mentions.length > 0 ? [{label: 'Mentioned', value: mentions.join(', ')}] : []),
  ];
  const sizeActions = new Set(options.map(o => actionFor(block, o.size)?.action_id).filter(Boolean));
  const rest = {...block, actions: block.actions.filter(a => !sizeActions.has(a.action_id))};
  const productFacts = [...list(product.facts), moneyText(product.price)].filter(Boolean).join(' · ');

  return (
    <BlockFrame block={block} footer={<BlockActions block={rest} />}>
      {product.name ? (
        <HStack gap={3} vAlign="center">
          <Thumbnail src={imageUrl(product.image as ImageRef) ?? undefined} alt={product.name} label={product.name} />
          <VStack gap={0.5}>
            {product.product_id ? (
              <Link href={`/p/${encodeURIComponent(product.product_id)}`}>
                <Text type="body" weight="semibold">
                  {product.name}
                </Text>
              </Link>
            ) : (
              <Text type="body" weight="semibold">
                {product.name}
              </Text>
            )}
            {productFacts ? (
              <Text type="supporting" color="secondary">
                {productFacts}
              </Text>
            ) : null}
            {product.note || product.summary ? (
              <Text type="supporting" color="secondary">
                {product.note ?? product.summary}
              </Text>
            ) : null}
          </VStack>
        </HStack>
      ) : null}
      {options.length > 0 ? (
        <Grid columns={{minWidth: 220}} gap={3}>
          {options.map((o, i) => (
            <OptionCard key={o.size} block={block} option={o} isFirst={i === 0} />
          ))}
        </Grid>
      ) : null}
      {data.measurements && list(data.measurements.rows as unknown[]).length > 0 ? <Measurements block={block} m={data.measurements} /> : null}
      <Grid columns={{minWidth: 260}} gap={3}>
        <Facts title="From reviews" aside={reviews.sample !== null && reviews.sample !== undefined ? textOf(reviews.sample) : undefined} items={reviewItems} />
        {used.length + notUsed.length > 0 ? (
          <Card>
            <VStack gap={2}>
              <HStack gap={2} justify="between" vAlign="center" wrap="wrap">
                <Text type="label" weight="semibold">
                  What I used
                </Text>
                <Text type="supporting" color="secondary">
                  you can ask me to leave any out
                </Text>
              </HStack>
              <MetadataList>
                {used.map((u, i) => (
                  <UsedFact key={`u-${i}`} block={block} label={u.label} value={u.value} used />
                ))}
                {notUsed.map((u, i) => (
                  <UsedFact key={`n-${i}`} block={block} label={u.label} value={u.value} used={false} />
                ))}
              </MetadataList>
            </VStack>
          </Card>
        ) : null}
      </Grid>
    </BlockFrame>
  );
}

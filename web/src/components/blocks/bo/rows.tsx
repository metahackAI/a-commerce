'use client';

// A table of loosely shaped rows from a block (a change group's products, a product table): the
// columns are the rows' own fields, each value formatted as sent, each row an AI target.

import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {VStack} from '@astryxdesign/core/Layout';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';

import {price, value, words} from '@/app/bo/_lib/format';
import {entityHref} from '@/app/bo/_lib/links';
import type {Change} from '@/app/bo/_lib/types';
import {changeSubject, type IntentsFor} from '@/components/bo/ChangeParts';
import {AskHeaderCell, TargetRow} from '@/components/bo/targets';

const ID_KEYS = ['product_id', 'variant_id', 'claim_id', 'order_id', 'promotion_id', 'customer_id', 'id', 'draft_id'];
const NAME_KEYS = ['name', 'title', 'label', 'item'];
const HIDDEN = new Set(['price_currency', 'currency', 'image', 'image_url', '@type']);

/** The first of `keys` the row has as non-empty text (a change row's `id` is "" before it runs). */
function firstText(row: Record<string, unknown>, keys: string[]): string | null {
  for (const k of keys) {
    const v = row[k];
    if (typeof v === 'string' && v.trim() !== '') return v;
  }
  return null;
}

/** The id a row is about, if it carries one. */
export function rowId(row: Record<string, unknown>): string | null {
  return firstText(row, ID_KEYS);
}

function rowName(row: Record<string, unknown>): string | null {
  return firstText(row, NAME_KEYS);
}

/**
 * A short name for a row picked as "this": its name, or for a row without one, what it is about
 * rather than a bare id: a change as the changes table says it (`Update unit price specification ·
 * prd_windbreaker`), anything else by its type (`Question · Packable windbreaker`).
 */
function rowTitle(row: Record<string, unknown>, name: string): string {
  if (rowName(row)) return name;
  if (typeof row.op === 'string' && typeof row.entity === 'string') {
    return `${words(row.op)} ${words(row.entity).toLowerCase()} · ${changeSubject(row as unknown as Change)}`;
  }
  const type = typeof row['@type'] === 'string' ? (row['@type'] as string).replace(/^acom:/, '') : null;
  const about = firstText(row, ['product_name']) ?? name;
  return type ? `${type} · ${about}` : about;
}

/** One value of a row as text; money-like numbers use the row's currency. */
export function cellText(row: Record<string, unknown>, key: string): string {
  const v = row[key];
  const currency = typeof row.price_currency === 'string' ? row.price_currency : typeof row.currency === 'string' ? row.currency : null;
  if (typeof v === 'number' && currency && /price|amount|cost|total|revenue|floor/.test(key)) return price(v, currency);
  if (typeof v === 'number' && /percent/.test(key)) return `${v}%`;
  return value(v);
}

export interface RowColumn {
  key: string;
  label: string;
}

/** Columns from the rows themselves: the name first, ids folded into it, then the other fields. */
export function columnsOf(rows: Record<string, unknown>[]): RowColumn[] {
  const keys: string[] = [];
  for (const r of rows) for (const k of Object.keys(r)) if (!keys.includes(k)) keys.push(k);
  return keys.filter(k => !HIDDEN.has(k) && !ID_KEYS.includes(k) && !NAME_KEYS.includes(k)).map(k => ({key: k, label: words(k)}));
}

export function RowsTable({
  rows,
  columns,
  kind,
  blockId,
  ids,
  intentsFor,
  empty = 'No rows',
}: {
  rows: Record<string, unknown>[];
  columns?: RowColumn[];
  kind: string;
  blockId?: string;
  ids?: Record<string, string>;
  intentsFor?: IntentsFor;
  empty?: string;
}) {
  if (rows.length === 0) return <EmptyState isCompact title={empty} />;
  const cols = columns ?? columnsOf(rows);
  const hasName = rows.some(r => rowName(r) || rowId(r));
  return (
    <Table density="compact" verticalAlign="top">
      <TableHeader>
        <TableRow isHeaderRow>
          {hasName ? <TableHeaderCell>Item</TableHeaderCell> : null}
          {cols.map(c => (
            <TableHeaderCell key={c.key}>{c.label}</TableHeaderCell>
          ))}
          <AskHeaderCell />
        </TableRow>
      </TableHeader>
      <TableBody>
        {rows.map((r, i) => {
          const id = rowId(r);
          const name = rowName(r) ?? id ?? `Row ${i + 1}`;
          const href = entityHref(id);
          const title = rowTitle(r, name);
          const label = [title, ...cols.map(c => `${c.label} ${cellText(r, c.key)}`)].join(' · ');
          return (
            <TargetRow
              key={id ?? i}
              target={{kind, label, title, ids: {[id ? `${id.slice(0, id.indexOf('_'))}_id` : 'row']: id ?? String(i), ...ids}, blockId}}
              intents={id ? intentsFor?.(kind, id) : undefined}>
              {hasName ? (
                <TableCell>
                  <VStack gap={0}>
                    {href ? <Link href={href}>{name}</Link> : <Text weight="medium">{name}</Text>}
                    {id && id !== name ? (
                      <Text type="supporting" color="secondary">
                        {id}
                      </Text>
                    ) : null}
                  </VStack>
                </TableCell>
              ) : null}
              {cols.map(c => (
                <TableCell key={c.key}>
                  <Text hasTabularNumbers>{cellText(r, c.key)}</Text>
                </TableCell>
              ))}
            </TargetRow>
          );
        })}
      </TableBody>
    </Table>
  );
}

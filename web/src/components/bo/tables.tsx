'use client';

// The back office's list tables. Each row is an AI target with an Ask button; the first cell
// links to the record. Values are shown as the API returned them, only formatted.

import type {ReactNode} from 'react';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {Thumbnail} from '@astryxdesign/core/Thumbnail';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';

import type {AiTarget} from '@/ai/useAiTarget';

import {AskHeaderCell, TargetRow} from './targets';

export interface Column<T> {
  header: string;
  /** Right-align figures. */
  numeric?: boolean;
  /** The column's share of the table, such as `30%`, for the column people read first. */
  width?: `${number}%`;
  cell: (row: T) => ReactNode;
}

/** A table of rows that are AI targets. */
export function DataTable<T>({
  rows,
  columns,
  rowKey,
  target,
  empty,
  intentsFor,
}: {
  rows: T[];
  columns: Column<T>[];
  rowKey: (row: T) => string;
  target: (row: T) => AiTarget;
  empty: {title: string; description?: string};
  intentsFor?: (row: T) => string[] | undefined;
}) {
  if (rows.length === 0) return <EmptyState isCompact title={empty.title} description={empty.description} />;
  return (
    <Table density="compact" verticalAlign="top" hasHover>
      <TableHeader>
        <TableRow isHeaderRow>
          {columns.map(c => (
            <TableHeaderCell key={c.header} style={c.width ? {width: c.width} : undefined}>
              {c.header}
            </TableHeaderCell>
          ))}
          <AskHeaderCell />
        </TableRow>
      </TableHeader>
      <TableBody>
        {rows.map(row => (
          <TargetRow key={rowKey(row)} target={target(row)} intents={intentsFor?.(row)}>
            {columns.map(c => (
              <TableCell key={c.header}>{c.cell(row)}</TableCell>
            ))}
          </TargetRow>
        ))}
      </TableBody>
    </Table>
  );
}

/**
 * The first cell of a row: the record's name as a link, with its id under it. A record with a
 * picture (a product) shows it before the name, as the Shopify admin lists products; `null` keeps
 * the place with an empty square so the names stay in line.
 */
export function RecordCell({href, name, id, image}: {href: string | null; name: string; id?: string; image?: string | null}) {
  const text = (
    <VStack gap={0}>
      {href ? (
        <Link href={href} color="primary" weight="medium">
          {name}
        </Link>
      ) : (
        <Text weight="medium">{name}</Text>
      )}
      {id && id !== name ? (
        <Text type="supporting" color="secondary">
          {id}
        </Text>
      ) : null}
    </VStack>
  );
  if (image === undefined) return text;
  // The name beside it says what it is, so the picture is decorative (alt=""). Small, as Polaris's
  // small thumbnail in a list row, instead of Thumbnail's attachment size.
  return (
    <HStack gap={3} vAlign="center">
      <Thumbnail src={image ?? undefined} alt="" style={{width: 'var(--spacing-10)'}} />
      {text}
    </HStack>
  );
}

/** A figure in a cell, with tabular digits. */
export function Figure({children, muted}: {children: ReactNode; muted?: boolean}) {
  return (
    <Text hasTabularNumbers color={muted ? 'secondary' : undefined}>
      {children}
    </Text>
  );
}

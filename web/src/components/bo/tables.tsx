'use client';

// The back office's list tables. Each row is an AI target with an Ask button; the first cell
// links to the record. Values are shown as the API returned them, only formatted.

import type {ReactNode} from 'react';
import {VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Link} from '@astryxdesign/core/Link';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {Table, TableBody, TableCell, TableHeader, TableHeaderCell, TableRow} from '@astryxdesign/core/Table';

import type {AiTarget} from '@/ai/useAiTarget';

import {TargetRow} from './targets';

export interface Column<T> {
  header: string;
  /** Right-align figures. */
  numeric?: boolean;
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
            <TableHeaderCell key={c.header}>{c.header}</TableHeaderCell>
          ))}
          <TableHeaderCell>
            <Text type="supporting" color="secondary">
              Ask
            </Text>
          </TableHeaderCell>
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

/** The first cell of a row: the record's name as a link, with its id under it. */
export function RecordCell({href, name, id}: {href: string | null; name: string; id?: string}) {
  return (
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
}

/** A figure in a cell, with tabular digits. */
export function Figure({children, muted}: {children: ReactNode; muted?: boolean}) {
  return (
    <Text hasTabularNumbers color={muted ? 'secondary' : undefined}>
      {children}
    </Text>
  );
}

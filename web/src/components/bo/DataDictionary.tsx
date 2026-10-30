'use client';

// The data dictionary (describe_data): each table with its schema.org term, one-sentence meaning,
// id prefix and API name, and every column with its term, type, allowed values and meaning.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Collapsible} from '@astryxdesign/core/Collapsible';
import {Section} from '@astryxdesign/core/Section';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {Token} from '@astryxdesign/core/Token';

import type {DataTable as Table} from '@/app/bo/_lib/types';

import {DataTable} from './tables';
import {TargetBlock} from './targets';

export function DataDictionary({tables, open}: {tables: Table[]; open: boolean}) {
  if (tables.length === 0) return <EmptyState title="Nothing matches" description="Try another term, schema or API name." />;
  return (
    <VStack gap={3}>
      {tables.map(t => (
        <Section key={t.table} padding={3}>
          <Collapsible
            defaultIsOpen={open}
            trigger={
              <HStack gap={2} vAlign="center" wrap="wrap">
                <Text weight="semibold">{t.table}</Text>
                <Token label={t.term} size="sm" />
                {t.api_name ? <Token label={`API: ${t.api_name}`} size="sm" color="blue" /> : null}
                {t.id_prefix ? <Token label={`${t.id_prefix}_…`} size="sm" color="gray" /> : null}
              </HStack>
            }>
            <VStack gap={3} paddingBlockStart={2}>
              <TargetBlock target={{kind: 'data_table', label: `${t.table} (${t.term}): ${t.description}`, ids: {table: t.table}, term: t.term}}>
                <Text>{t.description}</Text>
              </TargetBlock>
              <DataTable
                rows={t.columns}
                rowKey={c => c.name}
                empty={{title: 'No columns'}}
                target={c => ({kind: 'data_column', label: `${t.table}.${c.name} (${c.term}): ${c.description}`, ids: {column: `${t.table}.${c.name}`}, term: c.term})}
                columns={[
                  {
                    header: 'Column',
                    cell: c => (
                      <VStack gap={0}>
                        <Text weight="medium">{c.name}</Text>
                        <Text type="supporting" color="secondary">
                          {c.term}
                        </Text>
                      </VStack>
                    ),
                  },
                  {header: 'Type', cell: c => <Text color="secondary">{`${c.type}${c.nullable ? ', optional' : ''}`}</Text>},
                  {
                    header: 'Meaning',
                    cell: c => (
                      <VStack gap={0.5}>
                        <Text>{c.description}</Text>
                        {c.values && c.values.length > 0 ? (
                          <Text type="supporting" color="secondary">
                            {`Values: ${c.values.join(', ')}`}
                          </Text>
                        ) : null}
                        {c.references ? (
                          <Text type="supporting" color="secondary">
                            {`Points to ${c.references}`}
                          </Text>
                        ) : null}
                      </VStack>
                    ),
                  },
                  {header: 'API name', cell: c => <Text color="secondary">{c.api_name ?? '—'}</Text>},
                ]}
              />
            </VStack>
          </Collapsible>
        </Section>
      ))}
    </VStack>
  );
}

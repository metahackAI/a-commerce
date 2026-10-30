'use client';

// Filters of a list page, kept in the URL: the server page reads them as the tool's inputs, so a
// filtered list can be shared and reloaded. Choosing a value applies it; text applies on Enter.
// Any change starts again from the first page.

import {useState} from 'react';
import {usePathname, useRouter} from 'next/navigation';
import {HStack} from '@astryxdesign/core/Layout';
import {TextInput} from '@astryxdesign/core/TextInput';
import {Selector} from '@astryxdesign/core/Selector';
import {MultiSelector} from '@astryxdesign/core/MultiSelector';
import {DateInput} from '@astryxdesign/core/DateInput';
import type {ISODateString} from '@astryxdesign/core/Calendar';
import {Button} from '@astryxdesign/core/Button';

import {withParams} from '@/app/bo/_lib/links';

export interface FilterOption {
  value: string;
  label: string;
}

export type Filter =
  | {kind: 'text'; key: string; label: string; placeholder?: string; width?: number}
  | {kind: 'select'; key: string; label: string; options: FilterOption[]; placeholder?: string}
  | {kind: 'multi'; key: string; label: string; options: FilterOption[]; placeholder?: string}
  | {kind: 'date'; key: string; label: string};

export type FilterValues = Record<string, string | string[] | undefined>;

export function FilterBar({filters, values, keep}: {filters: Filter[]; values: FilterValues; keep?: FilterValues}) {
  const router = useRouter();
  const pathname = usePathname();
  const [text, setText] = useState<Record<string, string>>(() =>
    Object.fromEntries(filters.filter(f => f.kind === 'text').map(f => [f.key, String(values[f.key] ?? '')])),
  );

  const current: FilterValues = {...keep};
  for (const f of filters) current[f.key] = values[f.key];

  function apply(next: FilterValues) {
    router.push(withParams(pathname, {...current, ...next, cursor: undefined}));
  }

  const hasAny = filters.some(f => {
    const v = values[f.key];
    return Array.isArray(v) ? v.length > 0 : Boolean(v);
  });

  return (
    <HStack gap={3} vAlign="end" wrap="wrap">
      {filters.map(f => {
        switch (f.kind) {
          case 'text':
            return (
              <TextInput
                key={f.key}
                label={f.label}
                value={text[f.key] ?? ''}
                placeholder={f.placeholder}
                width={f.width ?? 240}
                size="sm"
                hasClear
                onChange={v => {
                  setText(t => ({...t, [f.key]: v}));
                  if (v === '' && values[f.key]) apply({[f.key]: undefined});
                }}
                onEnter={() => apply({[f.key]: text[f.key] || undefined})}
              />
            );
          case 'select':
            return (
              <Selector
                key={f.key}
                label={f.label}
                size="sm"
                width={200}
                options={f.options}
                value={typeof values[f.key] === 'string' ? (values[f.key] as string) : ''}
                placeholder={f.placeholder ?? 'Any'}
                hasClear
                onChange={(v: string | null) => apply({[f.key]: v ?? undefined})}
              />
            );
          case 'multi': {
            const v = values[f.key];
            const list = Array.isArray(v) ? v : v ? [v] : [];
            return (
              <MultiSelector
                key={f.key}
                label={f.label}
                size="sm"
                width={240}
                options={f.options}
                value={list}
                placeholder={f.placeholder ?? 'Any'}
                triggerDisplay="labels"
                hasClear
                onChange={next => apply({[f.key]: next})}
              />
            );
          }
          case 'date':
            return (
              <DateInput
                key={f.key}
                label={f.label}
                size="sm"
                width={180}
                value={typeof values[f.key] === 'string' ? (values[f.key] as ISODateString) : undefined}
                hasClear
                format="date"
                onChange={d => apply({[f.key]: d ?? undefined})}
              />
            );
          default:
            return null;
        }
      })}
      {hasAny ? (
        <Button
          label="Clear filters"
          size="sm"
          variant="ghost"
          onClick={() => {
            setText({});
            router.push(withParams(pathname, {...keep}));
          }}
        />
      ) : null}
    </HStack>
  );
}

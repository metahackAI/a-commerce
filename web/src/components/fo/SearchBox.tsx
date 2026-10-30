'use client';

// The search box with suggestions from `autocomplete` (search.md). The first row always searches
// for what was typed; the rest are the store's suggestions (keywords, products, brands,
// categories). It is an AI target so a shopper can ask the assistant to search in words instead.

import {Typeahead} from '@astryxdesign/core/Typeahead';
import type {SearchableItem, SearchSource} from '@astryxdesign/core/Typeahead';
import {MagnifyingGlassIcon} from '@heroicons/react/24/outline';
import {useRouter} from 'next/navigation';
import {useMemo, useRef, useState} from 'react';

import {useAiTarget} from '@/ai/useAiTarget';
import {api, type Page} from '@/lib/api';
import type {Suggestion} from '@/app/(storefront)/_lib/types';

interface SearchItem extends SearchableItem {
  href: string;
}

function hrefFor(s: Suggestion): string {
  if (s.suggest_type === 'product' && s.id?.startsWith('prd_')) return `/p/${encodeURIComponent(s.id)}`;
  if (s.suggest_type === 'category' && s.id?.startsWith('cat_')) return `/c/${encodeURIComponent(s.id)}`;
  return `/search?q=${encodeURIComponent(s.text)}`;
}

export function SearchBox({initial = '', width = '100%', autoFocus = false}: {initial?: string; width?: number | string; autoFocus?: boolean}) {
  const router = useRouter();
  const [query, setQuery] = useState(initial);
  const aborter = useRef<AbortController | null>(null);
  const target = useAiTarget({kind: 'search_box', label: 'Search box', ids: {input: 'search'}, term: 'schema:SearchAction'});

  const source = useMemo<SearchSource<SearchItem>>(
    () => ({
      async search(q: string) {
        const typed: SearchItem = {id: `q:${q}`, label: `Search for “${q}”`, href: `/search?q=${encodeURIComponent(q)}`};
        aborter.current?.abort();
        const ctl = new AbortController();
        aborter.current = ctl;
        try {
          const res = await api<Page<Suggestion>>('/v1/search/autocomplete', {query: {prefix: q, limit: 8}, signal: ctl.signal});
          const seen = new Set<string>();
          const items: SearchItem[] = [];
          for (const s of res.items) {
            const key = `${s.suggest_type}:${s.text}`;
            if (seen.has(key)) continue;
            seen.add(key);
            items.push({id: key, label: s.text, href: hrefFor(s)});
          }
          return [typed, ...items];
        } catch {
          // Suggestions are optional: searching for what was typed still works.
          return [typed];
        }
      },
      bootstrap() {
        return [];
      },
      cancel() {
        aborter.current?.abort();
      },
    }),
    [],
  );

  return (
    <Typeahead<SearchItem>
      ref={target.ref}
      {...target.props}
      label="Search the store"
      isLabelHidden
      placeholder="Search jackets, knits, boots…"
      startIcon={MagnifyingGlassIcon}
      searchSource={source}
      value={null}
      hasClear={false}
      hasAutoFocus={autoFocus}
      width={width}
      onChangeQuery={setQuery}
      onChange={item => {
        if (item) router.push(item.href);
      }}
      onKeyDown={e => {
        // Enter with the menu closed searches for the typed words.
        if (e.key === 'Enter' && query.trim() && !e.defaultPrevented) {
          router.push(`/search?q=${encodeURIComponent(query.trim())}`);
        }
      }}
    />
  );
}

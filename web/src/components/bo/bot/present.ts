// Ailism Bot's reads as people read them (docs/api/platform.md#bot): what each read tool looked
// at, with which arguments, what it found, and its items by name with a few figures. The raw
// result stays behind a toggle; nothing here changes a value, it only picks and words them.

import {dateTime, words} from '@/app/bo/_lib/format';

/** What each read the bot makes is about. */
const READ_LABEL: Record<string, string> = {
  search_products: 'Products',
  get_policy: 'Store policy',
  list_active_promotions: 'Promotions running',
  explain_sales_change: 'Why sales changed',
  list_display_slots: 'Main page slots',
  list_curations: 'Collections',
  search_return_requests: 'Return requests',
  list_questions: 'Product questions',
  list_reviews: 'Reviews',
  list_boards: 'Community boards',
  list_posts: 'Reported posts',
  list_tasks: 'Open tasks',
};

const SORT: Record<string, string> = {
  price: 'lowest price first',
  '-price': 'highest price first',
  inventory_level: 'least stock first',
  '-inventory_level': 'most stock first',
  newest: 'newest first',
  rating: 'lowest rating first',
  '-amount': 'largest first',
};

export function readLabel(tool: string, args: unknown): string {
  const base = READ_LABEL[tool] ?? words(tool.replace(/^(get|list|search)_/, ''));
  const a = (args ?? {}) as Record<string, unknown>;
  const sort = typeof a.sort === 'string' ? SORT[a.sort] : undefined;
  if (tool === 'get_policy' && typeof a.key === 'string') return `${base}: ${words(a.key)}`;
  if (tool === 'list_questions' && a.answered === false) return 'Questions without an answer';
  if (tool === 'list_posts' && a.reported === true) return `Reported posts on ${String(a.board_id ?? 'a board')}`;
  if (tool === 'search_return_requests') return 'Return requests waiting';
  return sort ? `${base}, ${sort}` : base;
}

/** The arguments in a line ("sort price · limit 30"). */
export function argsText(args: unknown): string {
  if (!args || typeof args !== 'object') return '';
  return Object.entries(args as Record<string, unknown>)
    .map(([k, v]) => `${words(k)} ${Array.isArray(v) ? v.join(', ') : typeof v === 'object' && v ? valueText(v) : String(v)}`)
    .join(' · ');
}

function isMoney(v: unknown): v is {value: number; currency: string} {
  return !!v && typeof v === 'object' && typeof (v as {value?: unknown}).value === 'number' && typeof (v as {currency?: unknown}).currency === 'string';
}

function valueText(v: unknown): string {
  if (isMoney(v)) return new Intl.NumberFormat('en-US', {style: 'currency', currency: v.currency}).format(v.value);
  if (typeof v === 'number') return Number.isInteger(v) ? v.toLocaleString('en-US') : v.toLocaleString('en-US', {maximumFractionDigits: 2});
  if (typeof v === 'boolean') return v ? 'yes' : 'no';
  if (typeof v === 'string') return /^\d{4}-\d{2}-\d{2}T/.test(v) ? dateTime(v) : words(v);
  return '';
}

function items(result: unknown): Record<string, unknown>[] {
  const r = (result ?? {}) as Record<string, unknown>;
  const list = Array.isArray(r.items) ? r.items : Array.isArray(r.lines) ? r.lines : null;
  return (list ?? []).filter((i): i is Record<string, unknown> => !!i && typeof i === 'object');
}

/** "30 of 382 found", "1 found", "nothing found", or "read" for one record. */
export function readOutcome(result: unknown, error?: string): string {
  if (error) return 'could not be read';
  const r = (result ?? {}) as Record<string, unknown>;
  const list = items(result);
  if (!Array.isArray(r.items) && !Array.isArray(r.lines)) return 'read';
  const total = typeof r.total === 'number' ? r.total : list.length;
  if (total === 0) return 'nothing found';
  return list.length < total ? `${list.length} of ${total.toLocaleString('en-US')} shown` : `${total.toLocaleString('en-US')} found`;
}

/** The fields worth a glance, in this order, with how they are named. */
const FACTS: [string, string][] = [
  ['price', 'price'],
  ['cost', 'cost'],
  ['inventory_level', 'in stock'],
  ['availability', ''],
  ['status', ''],
  ['state', ''],
  ['amount', ''],
  ['review_rating', 'rating'],
  ['rating', 'rating'],
  ['reason', ''],
  ['report_count', 'reports'],
  ['valid_through', 'until'],
  ['date_created', ''],
  ['requested_at', 'requested'],
];

export interface ReadRow {
  key: string;
  title: string;
  facts: string[];
}

/** Up to `most` items of a read, each by its name with a few figures. */
export function readRows(result: unknown, most = 8): ReadRow[] {
  return items(result)
    .slice(0, most)
    .map((i, n) => {
      const title = ['name', 'headline', 'title', 'text', 'label', 'id'].map(k => i[k]).find(v => typeof v === 'string' && v.trim() !== '') as string | undefined;
      const facts = FACTS.flatMap(([k, label]) => {
        const v = i[k];
        if (v === undefined || v === null || v === '') return [];
        const t = valueText(v);
        return t ? [label ? `${label} ${t}` : t] : [];
      }).slice(0, 4);
      return {key: String(i.id ?? i.variant_id ?? n), title: title ?? `Item ${n + 1}`, facts};
    });
}

/** For a read that returned one record (a policy, the sales explanation): its plain fields. */
export function recordFacts(result: unknown, most = 6): string[] {
  if (!result || typeof result !== 'object' || Array.isArray((result as {items?: unknown}).items)) return [];
  return Object.entries(result as Record<string, unknown>)
    .filter(([, v]) => v !== null && (typeof v !== 'object' || isMoney(v)))
    .map(([k, v]) => `${words(k)}: ${valueText(v)}`)
    .filter(s => !s.endsWith(': '))
    .slice(0, most);
}

/** "48 s" between two store times. */
export function took(from: string, to: string | null): string | null {
  if (!to) return null;
  const s = Math.round((new Date(to).getTime() - new Date(from).getTime()) / 1000);
  if (!Number.isFinite(s) || s < 0) return null;
  return s < 90 ? `${s} s` : `${Math.round(s / 60)} min`;
}

// Display formatting only. Every number shown here comes from an API response as it is; these
// helpers change how it reads (currency sign, date words), never what it is.

import type {ImageRef, Money} from './types';

const moneyFormats = new Map<string, Intl.NumberFormat>();

function moneyFormat(currency: string): Intl.NumberFormat {
  let f = moneyFormats.get(currency);
  if (!f) {
    try {
      f = new Intl.NumberFormat('en-US', {style: 'currency', currency});
    } catch {
      f = new Intl.NumberFormat('en-US', {minimumFractionDigits: 2, maximumFractionDigits: 2});
    }
    moneyFormats.set(currency, f);
  }
  return f;
}

/** `{value: 71.2, currency: "USD"}` → `$71.20`. */
export function money(m: Money | null | undefined): string {
  if (!m || typeof m.value !== 'number') return '—';
  return moneyFormat(m.currency || 'USD').format(m.value);
}

/** A flattened schema.org price (`price` + `price_currency`) → `$89.00`. */
export function price(value: number | null | undefined, currency: string | null | undefined): string {
  if (typeof value !== 'number') return '—';
  return moneyFormat(currency || 'USD').format(value);
}

/**
 * Any money-like value a block may carry: `{value, currency}`, `{price, price_currency}`, a bare
 * number (in `fallbackCurrency`), or text the agent already formatted.
 */
export function anyMoney(v: unknown, fallbackCurrency = 'USD'): string {
  if (v === null || v === undefined) return '';
  if (typeof v === 'string') return v;
  if (typeof v === 'number') return price(v, fallbackCurrency);
  if (typeof v === 'object') {
    const o = v as Record<string, unknown>;
    if (typeof o.value === 'number') return price(o.value, (o.currency as string) ?? fallbackCurrency);
    if (typeof o.price === 'number') return price(o.price, (o.price_currency as string) ?? fallbackCurrency);
    if (typeof o.display === 'string') return o.display;
  }
  return '';
}

/** Whether a money value is exactly zero (to say "Free" for shipping). */
export function isZero(m: Money | null | undefined): boolean {
  return !!m && m.value === 0;
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

interface WallClock {
  y: number;
  m: number;
  d: number;
  hh: number | null;
  mm: number | null;
}

/** Reads the store's wall clock from `2026-11-07T10:00:00-05:00` or `2026-11-10` as written. */
function wallClock(s: string | null | undefined): WallClock | null {
  if (!s) return null;
  const match = /^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2}))?/.exec(s);
  if (!match) return null;
  return {
    y: Number(match[1]),
    m: Number(match[2]),
    d: Number(match[3]),
    hh: match[4] === undefined ? null : Number(match[4]),
    mm: match[5] === undefined ? null : Number(match[5]),
  };
}

function weekday(w: WallClock): string {
  return WEEKDAYS[new Date(Date.UTC(w.y, w.m - 1, w.d)).getUTCDay()];
}

function clock(w: WallClock): string {
  if (w.hh === null || w.mm === null) return '';
  const h12 = w.hh % 12 === 0 ? 12 : w.hh % 12;
  const ampm = w.hh < 12 ? 'AM' : 'PM';
  return w.mm === 0 ? `${h12} ${ampm}` : `${h12}:${String(w.mm).padStart(2, '0')} ${ampm}`;
}

/** `2026-11-10` → `Tue, Nov 10`. */
export function day(s: string | null | undefined): string {
  const w = wallClock(s);
  if (!w) return s ?? '';
  return `${weekday(w)}, ${MONTHS[w.m - 1]} ${w.d}`;
}

/** `2026-10-27T19:12:00-04:00` → `Oct 27, 2026`. */
export function longDate(s: string | null | undefined): string {
  const w = wallClock(s);
  if (!w) return s ?? '';
  return `${MONTHS[w.m - 1]} ${w.d}, ${w.y}`;
}

/** `2026-11-07T10:00:00-05:00` → `Sat, Nov 7, 10 AM` (store time, as written). */
export function dateTime(s: string | null | undefined): string {
  const w = wallClock(s);
  if (!w) return s ?? '';
  const t = clock(w);
  return t ? `${weekday(w)}, ${MONTHS[w.m - 1]} ${w.d}, ${t}` : day(s);
}

/** A pickup window `{start, end}` → `Mon, Nov 9, 9 AM – 12 PM`. */
export function timeWindow(start: string | null | undefined, end: string | null | undefined): string {
  const a = wallClock(start);
  const b = wallClock(end);
  if (!a) return '';
  if (!b) return dateTime(start);
  const sameDay = a.y === b.y && a.m === b.m && a.d === b.d;
  return sameDay ? `${dateTime(start)} – ${clock(b)}` : `${dateTime(start)} – ${dateTime(end)}`;
}

const LABELS: Record<string, string> = {
  in_stock: 'In stock',
  out_of_stock: 'Out of stock',
  pre_order: 'Pre-order',
  back_order: 'Back order',
  payment_due: 'Payment due',
  in_transit: 'On the way',
  label_created: 'Label created',
  picked_up: 'Picked up',
  delivery_failed: 'Delivery failed',
  ship_instructed: 'Preparing to ship',
  awaiting_stock: 'Waiting for stock',
  true_to_size: 'True to size',
  runs_small: 'Runs small',
  runs_large: 'Runs large',
  store_automatic: 'Confirmed automatically',
  store_person: 'A person at the store confirms it',
  self_service: 'You can cancel this yourself',
  store_review: 'The store reviews this cancellation',
  not_allowed: 'Cannot be cancelled',
  free_return: 'Free returns',
  return_fees_customer_responsibility: 'Return shipping is on you',
  return_shipping_fees: 'Return shipping fee applies',
  drop_off: 'Drop off',
  credit_card: 'Card',
  bank_transfer: 'Bank transfer',
  virtual_account: 'Virtual account',
  loyalty_points: 'Loyalty points',
  store_credit: 'Store credit',
  payment_complete: 'Paid',
  customer_fault: 'Your choice',
  store_fault: 'Store issue',
  vip: 'VIP',
};

/** An enum word from docs/api/enums.md → words a shopper reads: `in_transit` → `On the way`. */
export function words(value: string | null | undefined): string {
  if (!value) return '';
  if (LABELS[value]) return LABELS[value];
  const s = value.replace(/_/g, ' ');
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/** The first image URL in an `image` field, whatever its shape. */
export function imageUrl(image: ImageRef): string | null {
  if (!image) return null;
  if (typeof image === 'string') return image;
  if (Array.isArray(image)) {
    for (const i of image) {
      const u = imageUrl(i);
      if (u) return u;
    }
    return null;
  }
  return image.content_url ?? image.url ?? null;
}

/** A category route slug from an id or term code: `cat_new_in` → `cat_new_in`, `new-in` → `cat_new_in`. */
export function categoryIdFromSlug(slug: string): string {
  const s = decodeURIComponent(slug);
  return s.startsWith('cat_') ? s : `cat_${s.replace(/-/g, '_')}`;
}

// Display formatting only. Every number shown here comes from the API as is; these helpers turn
// it into text (currency symbol, thousands separators, a readable date) and never compute a
// business value. Times keep the store offset the API sends (docs/api/conventions.md §5): the
// wall-clock part of the RFC 3339 string is shown, not converted to the viewer's time zone.

import type {Money} from './types';

const currencyFormats = new Map<string, Intl.NumberFormat>();

function currencyFormat(currency: string): Intl.NumberFormat {
  let f = currencyFormats.get(currency);
  if (!f) {
    try {
      f = new Intl.NumberFormat('en-US', {style: 'currency', currency});
    } catch {
      f = new Intl.NumberFormat('en-US', {minimumFractionDigits: 2, maximumFractionDigits: 2});
    }
    currencyFormats.set(currency, f);
  }
  return f;
}

/** `{value: 71.2, currency: "USD"}` → `$71.20`. */
export function money(m: Money | null | undefined): string {
  if (!m || typeof m.value !== 'number') return '—';
  return currencyFormat(m.currency).format(m.value);
}

/** A flattened price (`price` + `price_currency`) → `$89.00`. */
export function price(value: number | null | undefined, currency: string | null | undefined): string {
  if (typeof value !== 'number') return '—';
  return currencyFormat(currency ?? 'USD').format(value);
}

const integer = new Intl.NumberFormat('en-US');

/** `1284` → `1,284`. */
export function count(n: number | null | undefined): string {
  return typeof n === 'number' ? integer.format(n) : '—';
}

/** A 0–1 rate from a tool as a signed percent: `-0.2035` → `−20%`. */
export function percent(rate: number | null | undefined): string {
  if (typeof rate !== 'number') return '—';
  const whole = Math.round(Math.abs(rate) * 100);
  if (whole === 0) return '0%';
  return `${rate < 0 ? '−' : '+'}${whole}%`;
}

/** Whether a value looks like `{value, currency}`. */
export function isMoney(v: unknown): v is Money {
  return (
    typeof v === 'object' &&
    v !== null &&
    typeof (v as Money).value === 'number' &&
    typeof (v as Money).currency === 'string'
  );
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];
const STAMP = /^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2}))?/;

/** `2026-11-06T11:02:10-05:00` → `Nov 6, 2026, 11:02`; `2026-10-31` → `Oct 31, 2026`. */
export function dateTime(s: string | null | undefined): string {
  if (!s) return '—';
  const m = STAMP.exec(s);
  if (!m) return s;
  const [, y, mo, d, h, mi] = m;
  const day = `${MONTHS[Number(mo) - 1]} ${Number(d)}, ${y}`;
  return h ? `${day}, ${h}:${mi}` : day;
}

const WEEKDAYS = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];

/** A clock's face, the weekday first and no year: `2026-11-06T11:02:10-05:00` → `Fri Nov 6, 11:02`. */
export function clockFace(s: string | null | undefined): string {
  if (!s) return '—';
  const m = STAMP.exec(s);
  if (!m) return s;
  const [, y, mo, d, h, mi] = m;
  // The stamp's own calendar date, so the weekday doesn't move with the viewer's time zone.
  const weekday = WEEKDAYS[new Date(Date.UTC(Number(y), Number(mo) - 1, Number(d))).getUTCDay()];
  const day = `${weekday} ${MONTHS[Number(mo) - 1]} ${Number(d)}`;
  return h ? `${day}, ${h}:${mi}` : day;
}

/** The date part only: `Nov 6, 2026`. */
export function date(s: string | null | undefined): string {
  if (!s) return '—';
  const m = STAMP.exec(s);
  if (!m) return s;
  const [, y, mo, d] = m;
  return `${MONTHS[Number(mo) - 1]} ${Number(d)}, ${y}`;
}

/** A period: `Nov 7, 2026, 00:00 – Nov 8, 2026, 23:59`, or `from Nov 1, 2026` when open-ended. */
export function period(from: string | null | undefined, through: string | null | undefined): string {
  if (!from && !through) return '—';
  if (!through) return `From ${dateTime(from)}`;
  if (!from) return `Until ${dateTime(through)}`;
  return `${dateTime(from)} – ${dateTime(through)}`;
}

/** `waiting_for_approval` → `Waiting for approval`. A word from docs/api/enums.md made readable. */
export function words(s: string | null | undefined): string {
  if (!s) return '—';
  const text = s.replace(/^acom:/, '').replace(/_/g, ' ');
  return text.charAt(0).toUpperCase() + text.slice(1);
}

/** An action-log row's `action_status` (schema.org ActionStatusType): a `potential` action is a preview. */
export function actionStatus(status: string | null | undefined): string {
  const said: Record<string, string> = {potential: 'Previewed', active: 'Running', completed: 'Completed', failed: 'Failed'};
  return (status && said[status]) ?? words(status);
}

/** A policy rule as a name: `reorder.max_units_per_line` → `Reorder: max units per line`. */
export function ruleName(rule: string): string {
  const [head, ...rest] = rule.split('.');
  return rest.length === 0 ? words(head) : `${words(head)}: ${rest.join(' ').replace(/_/g, ' ')}`;
}

/** A whole date or timestamp (`2026-11-13`, `2026-11-13T23:59:00-05:00`), not text that starts with one. */
const WHOLE_STAMP = /^\d{4}-\d{2}-\d{2}(?:T\d{2}:\d{2}(?::\d{2})?(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})?)?$/;

/** Any value from a change or a policy, as one line of text. Money, prices and dates are formatted. */
export function value(v: unknown): string {
  if (v === null || v === undefined) return '—';
  if (typeof v === 'string') return WHOLE_STAMP.test(v) ? dateTime(v) : v;
  if (typeof v === 'number') return integer.format(v);
  if (typeof v === 'boolean') return v ? 'Yes' : 'No';
  if (isMoney(v)) return money(v);
  if (Array.isArray(v)) return v.map(value).join(', ');
  if (typeof v === 'object') {
    const o = v as Record<string, unknown>;
    if (typeof o.price === 'number' && typeof o.price_currency === 'string') {
      const rest = Object.entries(o)
        .filter(([k]) => k !== 'price' && k !== 'price_currency')
        .map(([k, x]) => `${words(k)}: ${value(x)}`);
      return [price(o.price, o.price_currency), ...rest].join(' · ');
    }
    return Object.entries(o)
      .map(([k, x]) => `${words(k)}: ${value(x)}`)
      .join(' · ');
  }
  return String(v);
}

/** Who did something: a model, a person id, or a system id. */
export function actor(a: unknown): string {
  if (!a) return '—';
  if (typeof a === 'string') return a;
  if (typeof a === 'object') {
    const o = a as {name?: string; id?: string};
    return o.name ?? o.id ?? '—';
  }
  return String(a);
}

/** A postal address as one line. */
export function address(a: unknown): string {
  if (!a) return '—';
  if (typeof a === 'string') return a;
  const o = a as Record<string, string | undefined>;
  return [o.name, o.street_address, o.address_locality, o.address_region, o.postal_code, o.address_country]
    .filter(Boolean)
    .join(', ');
}

// Turning values the agent sent into text, for display only.

import {anyMoney} from '@/app/(storefront)/_lib/format';

/** A cell value as text: text and numbers as they are, `{display}`/`{label}`/`{text}` objects by that field. */
export function textOf(v: unknown): string {
  if (v === null || v === undefined) return '';
  if (typeof v === 'string') return v;
  if (typeof v === 'number') return String(v);
  if (typeof v === 'boolean') return v ? 'Yes' : 'No';
  if (Array.isArray(v)) return v.map(textOf).filter(Boolean).join(', ');
  if (typeof v === 'object') {
    const o = v as Record<string, unknown>;
    for (const key of ['display', 'label', 'text', 'name', 'title']) {
      if (typeof o[key] === 'string') return o[key] as string;
    }
    const money = anyMoney(v);
    if (money) return money;
    if ('value' in o) return textOf(o.value);
  }
  return '';
}

/** A money-like value as text, or '' when there is none. */
export function moneyText(v: unknown): string {
  return v === null || v === undefined ? '' : anyMoney(v);
}

/** A money-like value as `{value, currency}` when it carries a number, else null. */
export function amountOf(v: unknown, fallbackCurrency = 'USD'): {value: number; currency: string} | null {
  if (typeof v === 'number') return {value: v, currency: fallbackCurrency};
  if (v && typeof v === 'object') {
    const o = v as Record<string, unknown>;
    if (typeof o.value === 'number') return {value: o.value, currency: typeof o.currency === 'string' ? o.currency : fallbackCurrency};
    if (typeof o.price === 'number') {
      return {value: o.price, currency: typeof o.price_currency === 'string' ? o.price_currency : fallbackCurrency};
    }
  }
  return null;
}

/**
 * Props for ProductCard's price from a block's `price` (what the shopper pays) and optional
 * `list_price`: numbers go to PriceTag (an AI target); text the agent formatted is shown as is.
 */
export function cardPrice(priceValue: unknown, listPrice: unknown, label?: string | null) {
  const pays = amountOf(priceValue);
  const list = amountOf(listPrice);
  if (pays && list && list.value !== pays.value && list.currency === pays.currency) {
    return {price: list.value, member: pays.value, memberLabel: label ?? null, currency: pays.currency};
  }
  if (pays) return {price: pays.value, currency: pays.currency};
  const shown = moneyText(priceValue);
  const was = moneyText(listPrice);
  return {priceText: shown || null, listPriceText: was && was !== shown ? was : null};
}

/** Whether a money-like value is exactly zero (for "Free" shipping). */
export function isZeroMoney(v: unknown): boolean {
  if (typeof v === 'number') return v === 0;
  if (v && typeof v === 'object') {
    const o = v as Record<string, unknown>;
    if (typeof o.value === 'number') return o.value === 0;
    if (typeof o.price === 'number') return o.price === 0;
  }
  return false;
}

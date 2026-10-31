'use client';

// An API error as the shopper reads it: the sentence from the API (docs/api/errors.md) and the
// next steps that fit — the page the suggested next tool lives on, sign-in, or asking the
// assistant.

import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {HStack} from '@astryxdesign/core/Layout';

import type {ApiErrorBody} from '@/lib/api';

import {useSay} from './Ask';

/** Where a suggested next tool can be done by hand in the storefront. */
const NEXT_PAGE: Record<string, {label: string; href: string}> = {
  search_products: {label: 'Search the store', href: '/search'},
  get_product: {label: 'Search the store', href: '/search'},
  get_cart: {label: 'Open your cart', href: '/cart'},
  add_to_cart: {label: 'Keep shopping', href: '/search'},
  remove_cart_item: {label: 'Open your cart', href: '/cart'},
  update_cart_item: {label: 'Open your cart', href: '/cart'},
  get_my_profile: {label: 'Check your addresses', href: '/account'},
  list_addresses: {label: 'Check your addresses', href: '/account'},
  get_payment_methods: {label: 'Back to checkout', href: '/checkout'},
  get_my_orders: {label: 'Your orders', href: '/orders'},
  search_orders: {label: 'Your orders', href: '/orders'},
  get_order: {label: 'Your orders', href: '/orders'},
  list_categories: {label: 'All products', href: '/search'},
  get_preferences: {label: 'Your account', href: '/account'},
};

export function ErrorNotice({
  error,
  title,
  signIn,
  askAbout,
}: {
  error: ApiErrorBody;
  title?: string;
  /** Offer sign-in (for `unauthenticated` and steps only a signed-in shopper can take). */
  signIn?: boolean;
  /** A sentence the shopper can send to the assistant about this problem. */
  askAbout?: string;
}) {
  const say = useSay();
  const next = error.next_tool ? NEXT_PAGE[error.next_tool] : undefined;
  const needsSignIn = signIn || error.class === 'unauthenticated' || error.code === 'not_signed_in';
  const actions =
    next || needsSignIn || askAbout ? (
      <HStack gap={2} wrap="wrap">
        {needsSignIn ? <Button label="Sign in" size="sm" variant="primary" href="/login" /> : null}
        {next ? <Button label={next.label} size="sm" href={next.href} /> : null}
        {askAbout ? <Button label="Ask Ailism" size="sm" variant="ghost" onClick={() => say(askAbout)} /> : null}
      </HStack>
    ) : undefined;
  return (
    <Banner
      status={error.class === 'upstream_unavailable' || error.class === 'internal' ? 'error' : 'warning'}
      title={title ?? error.message}
      description={title ? error.message : undefined}
      endContent={actions}
    />
  );
}

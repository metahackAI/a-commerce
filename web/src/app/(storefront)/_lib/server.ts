// Server Component reads for the storefront. A read never throws into the page: it comes back as
// data or as the API's error (code, sentence, next tool) so the page can say what happened and
// offer the next step.
import 'server-only';

import {ApiError, type ApiErrorBody, type Query} from '@/lib/api';
import {serverApi, serverPost, serverSession} from '@/lib/server-api';
import type {Session} from '@/lib/session';

export type Loaded<T> = {ok: true; data: T} | {ok: false; status: number; error: ApiErrorBody};

const unreachable: ApiErrorBody = {
  code: 'upstream_unavailable',
  class: 'upstream_unavailable',
  message: 'The store could not be reached. Try again in a moment.',
  next_tool: null,
};

/** `GET /api{path}` with the shopper's cookies. */
export async function load<T>(path: string, query?: Query): Promise<Loaded<T>> {
  try {
    return {ok: true, data: await serverApi<T>(path, query, 'fo')};
  } catch (e) {
    if (e instanceof ApiError) return {ok: false, status: e.status, error: e.body};
    return {ok: false, status: 503, error: unreachable};
  }
}

/** A read whose input is a body (`POST /v1/community/posts/search`, no side effects). */
export async function loadBody<T>(path: string, body: unknown): Promise<Loaded<T>> {
  try {
    return {ok: true, data: await serverPost<T>(path, body, 'fo')};
  } catch (e) {
    if (e instanceof ApiError) return {ok: false, status: e.status, error: e.body};
    return {ok: false, status: 503, error: unreachable};
  }
}

/** The data, or `fallback` when the read failed (for parts of a page that can be left out). */
export function orElse<T>(loaded: Loaded<T>, fallback: T): T {
  return loaded.ok ? loaded.data : fallback;
}

export type Shopper = {
  session: Session;
  /** A signed-in shopper (not a guest, not signed out). */
  isShopper: boolean;
  isGuest: boolean;
  /** A signed-in operator: shops as their browser session; the back office is one link away. */
  isOperator: boolean;
};

export async function shopper(): Promise<Shopper> {
  const session = await serverSession();
  const kind = session.signed_in ? session.user?.kind : undefined;
  return {session, isShopper: kind === 'shopper', isGuest: kind === 'guest', isOperator: kind === 'operator'};
}

/** The name to greet: a signed-in shopper's from their profile, an operator's from the session. */
export async function greetingName(who: Shopper): Promise<string | null> {
  if (who.isShopper) {
    const profile = await load<{name?: string | null}>('/v1/customer/people/me', {fields: ['name']});
    return profile.ok ? (profile.data.name ?? null) : null;
  }
  return who.isOperator ? (who.session.user?.name ?? null) : null;
}

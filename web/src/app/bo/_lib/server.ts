// Server-side reads for back-office pages. Pages read through `serverApi` (it forwards the
// browser's cookies to the gateway) and get either the data or the API's error, so a page can
// show the error's sentence and the suggested next step instead of failing as a whole.
import 'server-only';

import {cache} from 'react';
import {redirect} from 'next/navigation';

import {ApiError, type ApiErrorBody, type Query} from '@/lib/api';
import {serverApi, serverSession} from '@/lib/server-api';
import type {Session} from '@/lib/session';

export type Result<T> = {ok: true; data: T} | {ok: false; error: ApiErrorBody; status: number};

/** One read; an API error (or an unreachable gateway) comes back as a value. */
export async function read<T>(path: string, query?: Query): Promise<Result<T>> {
  try {
    return {ok: true, data: await serverApi<T>(path, query)};
  } catch (e) {
    if (e instanceof ApiError) return {ok: false, error: e.body, status: e.status};
    return {
      ok: false,
      status: 503,
      error: {
        code: 'upstream_unavailable',
        class: 'upstream_unavailable',
        message: 'The store could not be reached. Try again in a moment.',
        next_tool: null,
      },
    };
  }
}

/** The data when the read worked, otherwise `null` (for optional parts of a page). */
export async function readOptional<T>(path: string, query?: Query): Promise<T | null> {
  const r = await read<T>(path, query);
  return r.ok ? r.data : null;
}

/** The session, read once per request and shared by the layout and the page. */
export const boSession = cache(async (): Promise<Session> => serverSession());

/** Whether this session belongs to an operator. */
export function isOperator(session: Session): boolean {
  return session.signed_in && session.user?.kind === 'operator';
}

/** The session of a signed-in operator; anyone else goes to sign-in. */
export async function requireOperator(): Promise<Session> {
  const session = await boSession();
  if (!isOperator(session)) redirect('/bo/login');
  return session;
}

/** Search params as Next.js hands them to a page. */
export type SearchParams = Promise<Record<string, string | string[] | undefined>>;

/** One search param as a string (the first, when repeated). */
export function param(sp: Record<string, string | string[] | undefined>, key: string): string | undefined {
  const v = sp[key];
  const s = Array.isArray(v) ? v[0] : v;
  return s === '' ? undefined : s;
}

/** A repeated search param as a list (`?state=a&state=b`). Commas also split. */
export function params(sp: Record<string, string | string[] | undefined>, key: string): string[] {
  const v = sp[key];
  const list = Array.isArray(v) ? v : v ? [v] : [];
  return list.flatMap(x => x.split(',')).filter(Boolean);
}

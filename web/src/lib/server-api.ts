// Server-side calls (Server Components, route handlers): the same API through the gateway's
// internal address, carrying the browser's cookies so the gateway sees the same session.
import 'server-only';

import {cookies} from 'next/headers';

import {ApiError, type ApiErrorBody, type Query, toQuery} from './api';
import type {Session} from './session';

function gateway(): string {
  return (process.env.ACOM_GATEWAY_INTERNAL_URL ?? 'http://127.0.0.1:7000').replace(/\/$/, '');
}

async function cookieHeader(): Promise<string> {
  const jar = await cookies();
  return jar
    .getAll()
    .map(c => `${c.name}=${c.value}`)
    .join('; ');
}

/** A read from a Server Component: `serverApi('/v1/catalog/product-groups/prd_x')`. */
export async function serverApi<T>(path: string, query?: Query): Promise<T> {
  const res = await fetch(`${gateway()}/api${path}${toQuery(query)}`, {
    headers: {accept: 'application/json', cookie: await cookieHeader()},
    cache: 'no-store',
  });
  const text = await res.text();
  const body: unknown = text ? JSON.parse(text) : null;
  if (!res.ok) {
    const err = (body as {error?: ApiErrorBody} | null)?.error ?? {
      code: 'upstream_unavailable',
      class: 'upstream_unavailable',
      message: `The request failed (${res.status}).`,
      next_tool: null,
    };
    throw new ApiError(res.status, err);
  }
  return body as T;
}

/**
 * A read whose input is a body (the analytics reads are `POST` without side effects), from a
 * Server Component. The gateway checks CSRF on every POST, so the session's token goes along.
 */
export async function serverPost<T>(path: string, body: unknown): Promise<T> {
  const jar = await cookies();
  const csrf = jar.get('acom_csrf')?.value;
  const res = await fetch(`${gateway()}/api${path}`, {
    method: 'POST',
    headers: {
      accept: 'application/json',
      'content-type': 'application/json',
      cookie: await cookieHeader(),
      ...(csrf ? {'x-csrf-token': decodeURIComponent(csrf)} : {}),
    },
    body: JSON.stringify(body),
    cache: 'no-store',
  });
  const text = await res.text();
  const parsed: unknown = text ? JSON.parse(text) : null;
  if (!res.ok) {
    const err = (parsed as {error?: ApiErrorBody} | null)?.error ?? {
      code: 'upstream_unavailable',
      class: 'upstream_unavailable',
      message: `The request failed (${res.status}).`,
      next_tool: null,
    };
    throw new ApiError(res.status, err);
  }
  return parsed as T;
}

/** Who is signed in, read on the server (for layouts that redirect to sign-in). */
export async function serverSession(): Promise<Session> {
  try {
    const res = await fetch(`${gateway()}/api/auth/session`, {
      headers: {accept: 'application/json', cookie: await cookieHeader()},
      cache: 'no-store',
    });
    if (res.ok) return (await res.json()) as Session;
  } catch {
    // The gateway is unreachable: treat as signed out.
  }
  return {signed_in: false, user: null, scopes: [], csrf_token: null, demo_mode: false};
}

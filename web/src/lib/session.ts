// Signing in and out through the gateway (docs/backend/gateway.md §6). Operators sign in with a
// login id and password; in demo mode, anyone from `/api/auth/personas`; shoppers may also
// continue as guests.

import {ApiError, type ApiErrorBody, writeHeaders} from './api';

export type UserKind = 'operator' | 'shopper' | 'guest';

export interface SessionUser {
  id: string | null;
  kind: UserKind;
  name: string | null;
  surface: 'bo' | 'fo' | 'mcp' | 'system';
}

export interface Session {
  signed_in: boolean;
  user: SessionUser | null;
  scopes: string[];
  csrf_token: string | null;
  expires_at?: string;
  demo_mode: boolean;
}

export interface Persona {
  persona: string;
  kind: UserKind;
  name: string;
  description: string | null;
  login_id?: string | null;
}

async function auth<T>(path: string, method: 'GET' | 'POST', body?: unknown): Promise<T> {
  const res = await fetch(`/api/auth/${path}`, {
    method,
    headers: method === 'GET' ? {accept: 'application/json'} : writeHeaders(),
    body: body === undefined ? undefined : JSON.stringify(body),
    credentials: 'same-origin',
  });
  const data: unknown = await res.json().catch(() => null);
  if (!res.ok) {
    const err = (data as {error?: ApiErrorBody} | null)?.error ?? {
      code: 'upstream_unavailable',
      class: 'upstream_unavailable',
      message: `Sign-in failed (${res.status}).`,
      next_tool: null,
    };
    throw new ApiError(res.status, err);
  }
  return data as T;
}

export const getSession = () => auth<Session>('session', 'GET');
export const signIn = (loginId: string, password: string) =>
  auth<Session>('login', 'POST', {login_id: loginId, password});
export const demoSignIn = (persona: string) => auth<Session>('demo-login', 'POST', {persona});
export const continueAsGuest = () => auth<Session>('guest', 'POST');
export const signOut = () => auth<{ok: boolean}>('logout', 'POST');
export const listPersonas = () => auth<{items: Persona[]; total: number}>('personas', 'GET');

/** Whether the session holds a scope (`human:approve`, `tools:write:promotion`, …). */
export function can(session: Session | null, scope: string): boolean {
  return session?.scopes.includes(scope) ?? false;
}

'use client';

// The signed-in operator's session for client components (blocks decide from its scopes whether
// to show a person's buttons). The console layout reads it on the server and hands it down.

import {createContext, useContext, type ReactNode} from 'react';

import type {Session} from '@/lib/session';

const SessionContext = createContext<Session | null>(null);

export function BoSessionProvider({session, children}: {session: Session; children: ReactNode}) {
  return <SessionContext.Provider value={session}>{children}</SessionContext.Provider>;
}

export function useBoSession(): Session | null {
  return useContext(SessionContext);
}

/** The scope a person-only endpoint needs (`human:*`, docs/api/README.md §2.1; `admin:demo` for the demo clock, §2.2), or `null`. */
export function personScope(method: string, path: string): string | null {
  if (method !== 'POST' && method !== 'PUT') return null;
  if (/\/change-sets\/[^/]+\/approve$/.test(path)) return 'human:approve';
  if (/\/change-sets\/[^/]+\/reject$/.test(path)) return 'human:reject';
  if (/\/change-sets\/[^/]+\/undo$/.test(path)) return 'human:undo';
  if (/\/tasks\/[^/]+\/dismiss$/.test(path)) return 'human:task';
  if (/\/platform\/demo-clock$/.test(path)) return 'admin:demo';
  return null;
}

/** Whether this session may press a person button for `method path`. Unknown paths are left to the server. */
export function mayPress(session: Session | null, method: string, path: string): boolean {
  const scope = personScope(method, path);
  return scope === null || (session?.scopes.includes(scope) ?? false);
}

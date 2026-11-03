'use client';

// What a change set is now, for a chat block that showed it. A block is what the agent sent at the
// time; the set moves on when someone approves it (from the block, on its page, or elsewhere), so
// the block's state, buttons and undo follow `get_change_set`, not the snapshot. Read when the
// block appears or the agent replaces it, and when the window comes back; after a button on the
// block, followed until the set settles.

import {useCallback, useEffect, useRef, useState} from 'react';

import {api} from '@/lib/api';

export interface LiveChangeSet {
  state: string;
  version: number;
  undo_until: string | null;
}

/** States the runner moves on from by itself. */
const MOVING = new Set(['running', 'verifying']);
const EVERY_MS = 1500;
/** How long a press is followed before the block waits for the window to come back instead. */
const FOLLOW_MS = 120_000;

export function useLiveChangeSet(id: string, shown: LiveChangeSet): {live: LiveChangeSet; follow: () => void} {
  const [live, setLive] = useState<LiveChangeSet>(shown);
  const timer = useRef<number | undefined>(undefined);
  const gone = useRef(false);

  const read = useCallback(async (): Promise<LiveChangeSet | null> => {
    try {
      const cs = await api<LiveChangeSet>(`/v1/platform/change-sets/${encodeURIComponent(id)}`, {
        query: {fields: ['state', 'version', 'undo_until']},
      });
      if (!gone.current) setLive({state: cs.state, version: cs.version, undo_until: cs.undo_until ?? null});
      return cs;
    } catch {
      // The snapshot stays; the server still refuses a button that no longer fits.
      return null;
    }
  }, [id]);

  // The agent replaces a block while its turn runs (drafting, then waiting for approval): a new
  // snapshot is what the set was a moment ago, so it shows at once and is read again.
  const {state, version, undo_until} = shown;
  useEffect(() => {
    setLive({state, version, undo_until});
    void read();
  }, [state, version, undo_until, read]);

  useEffect(() => {
    gone.current = false;
    const again = () => {
      if (document.visibilityState === 'visible') void read();
    };
    window.addEventListener('focus', again);
    document.addEventListener('visibilitychange', again);
    return () => {
      gone.current = true;
      window.clearTimeout(timer.current);
      window.removeEventListener('focus', again);
      document.removeEventListener('visibilitychange', again);
    };
  }, [read]);

  // After a press: read until the set has left the state it was in and is not still running.
  const follow = useCallback(() => {
    window.clearTimeout(timer.current);
    const from = live.state;
    const until = Date.now() + FOLLOW_MS;
    const step = async () => {
      const cs = await read();
      const settled = cs !== null && cs.state !== from && !MOVING.has(cs.state);
      if (!settled && !gone.current && Date.now() < until) timer.current = window.setTimeout(step, EVERY_MS);
    };
    void step();
  }, [live.state, read]);

  return {live, follow};
}

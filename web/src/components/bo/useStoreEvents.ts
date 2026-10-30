'use client';

// The store's progress events (`GET /v1/platform/events`, docs/api/platform.md#내부-엔드포인트):
// `task.updated` when a watcher or a change set moves a task, `job.updated` when scheduled work
// runs, `change_set.updated` as a set goes through approval. Pages use them to reload their data;
// the numbers still come from the reads, never from the event.

import {useEffect, useRef} from 'react';

/** Calls `onEvent` (at most once per quiet half second) when one of `kinds` arrives. */
export function useStoreEvents(kinds: string[], onEvent: (kind: string) => void) {
  const handler = useRef(onEvent);
  useEffect(() => {
    handler.current = onEvent;
  });
  const key = kinds.join(',');

  useEffect(() => {
    if (typeof EventSource === 'undefined' || key === '') return;
    const source = new EventSource('/api/v1/platform/events', {withCredentials: true});
    let timer: number | undefined;
    const listener = (e: MessageEvent) => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => handler.current(e.type), 500);
    };
    const names = key.split(',');
    for (const name of names) source.addEventListener(name, listener);
    return () => {
      window.clearTimeout(timer);
      for (const name of names) source.removeEventListener(name, listener);
      source.close();
    };
  }, [key]);
}

// The chat over one WebSocket per page and surface (docs/api/chat.md §2.6), with the HTTP event
// stream (§2) as the fallback where a socket cannot be opened.
//
// Handshake: the page needs a session (a guest one is opened when there is none), opens
// `/api/chat/ws`, sends `hello` with the session's CSRF token, and waits for `ready`. Each turn
// then goes out as a `turn` frame with its own `request_id`; its events come back as `event`
// frames as they happen, and `end` closes it. `cancel` stops a turn. The socket is opened on the
// first turn, kept warm with pings, and opened again after it drops; a turn that was running
// when it dropped ends with an error the person can retry (nothing is sent twice by itself).

import {csrfToken, writeHeaders} from '@/lib/api';

import {sendTurn, type TurnRequest} from './stream';
import type {StreamEvent, Surface} from './types';

/** What carries the conversation right now (shown as `data-transport` on the workspace). */
export type Transport = 'idle' | 'connecting' | 'socket' | 'stream';

const PROTOCOL = 1;
const READY_MS = 8000;
const PING_MS = 25_000;
/** After the socket could not be opened, turns use the event stream for this long. */
const FALLBACK_MS = 60_000;

type Frame =
  | {type: 'ready'; protocol: number; surface: Surface; heartbeat_seconds?: number}
  | {type: 'event'; request_id: string; id?: number; event: string; data: unknown}
  | {type: 'end'; request_id: string; reason: 'done' | 'cancelled' | 'error'}
  | {type: 'error'; request_id?: string; code: string; message: string}
  | {type: 'pong'};

interface Pending {
  push: (e: StreamEvent | null) => void;
}

let counter = 0;
const nextRequestId = () => `r_${Date.now().toString(36)}${(counter++).toString(36)}`;

/** A queue an async generator can wait on. */
function channel<T>() {
  const items: T[] = [];
  let wake: (() => void) | null = null;
  return {
    push(item: T) {
      items.push(item);
      wake?.();
      wake = null;
    },
    async next(): Promise<T> {
      while (items.length === 0) await new Promise<void>(res => (wake = res));
      return items.shift() as T;
    },
  };
}

class ChatSocket {
  private ws: WebSocket | null = null;
  private opening: Promise<WebSocket> | null = null;
  private pending = new Map<string, Pending>();
  private ping: ReturnType<typeof setInterval> | null = null;
  private failedAt = 0;
  private listeners = new Set<(t: Transport) => void>();
  transport: Transport = 'idle';

  constructor(private readonly surface: Surface) {}

  onTransport(fn: (t: Transport) => void): () => void {
    this.listeners.add(fn);
    fn(this.transport);
    return () => this.listeners.delete(fn);
  }

  private setTransport(t: Transport) {
    this.transport = t;
    this.listeners.forEach(fn => fn(t));
  }

  /** Whether the socket should be tried now (not right after it failed). */
  usable(): boolean {
    return typeof WebSocket !== 'undefined' && Date.now() - this.failedAt > FALLBACK_MS;
  }

  failed() {
    this.failedAt = Date.now();
    this.setTransport('stream');
  }

  /** A turn goes over the event stream (no socket in this browser, or it failed lately). */
  streaming() {
    if (this.transport !== 'stream') this.setTransport('stream');
  }

  /** The open, ready socket: opens and greets one when there is none. */
  async open(): Promise<WebSocket> {
    if (this.ws && this.ws.readyState === WebSocket.OPEN) return this.ws;
    if (this.opening) return this.opening;
    this.setTransport('connecting');
    this.opening = this.connect().finally(() => (this.opening = null));
    return this.opening;
  }

  private async connect(): Promise<WebSocket> {
    // The upgrade needs a session: a guest one when the person has none yet.
    if (!csrfToken()) {
      await fetch('/api/auth/guest', {method: 'POST', headers: writeHeaders(), credentials: 'same-origin'}).catch(() => undefined);
    }
    const csrf = csrfToken();
    if (!csrf) throw new Error('no session');
    const url = `${window.location.protocol === 'https:' ? 'wss' : 'ws'}://${window.location.host}/api/chat/ws`;
    const ws = new WebSocket(url);
    await new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => {
        ws.close();
        reject(new Error('no ready'));
      }, READY_MS);
      ws.onopen = () => ws.send(JSON.stringify({type: 'hello', surface: this.surface, csrf_token: csrf, protocol: PROTOCOL}));
      ws.onerror = () => {
        clearTimeout(timer);
        reject(new Error('socket error'));
      };
      ws.onclose = () => {
        clearTimeout(timer);
        reject(new Error('closed during the handshake'));
      };
      ws.onmessage = m => {
        const f = JSON.parse(String(m.data)) as Frame;
        if (f.type === 'ready') {
          clearTimeout(timer);
          resolve();
        }
      };
    });
    ws.onmessage = m => this.receive(String(m.data));
    ws.onclose = () => this.dropped(ws);
    ws.onerror = () => undefined;
    this.ws = ws;
    this.ping = setInterval(() => {
      if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({type: 'ping'}));
    }, PING_MS);
    this.setTransport('socket');
    return ws;
  }

  private receive(raw: string) {
    let f: Frame;
    try {
      f = JSON.parse(raw) as Frame;
    } catch {
      return;
    }
    if (f.type === 'event') {
      this.pending.get(f.request_id)?.push({event: f.event, id: f.id === undefined ? undefined : String(f.id), data: f.data});
    } else if (f.type === 'error' && f.request_id) {
      this.pending.get(f.request_id)?.push({event: 'error', data: {code: f.code, message: f.message}});
    } else if (f.type === 'end') {
      this.pending.get(f.request_id)?.push(null);
      this.pending.delete(f.request_id);
    }
  }

  private dropped(ws: WebSocket) {
    if (this.ws !== ws) return;
    this.ws = null;
    if (this.ping) clearInterval(this.ping);
    this.ping = null;
    this.setTransport('idle');
    for (const [id, p] of this.pending) {
      p.push({event: 'error', data: {code: 'upstream_unavailable', message: 'The connection to the assistant dropped. Send it again.'}});
      p.push(null);
      this.pending.delete(id);
    }
  }

  /** One turn over the socket: its events as they come, until `end`. */
  async *turn(turn: TurnRequest, ws: WebSocket, signal?: AbortSignal): AsyncGenerator<StreamEvent> {
    const requestId = nextRequestId();
    const q = channel<StreamEvent | null>();
    this.pending.set(requestId, {push: q.push});
    const cancel = () => {
      if (ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify({type: 'cancel', request_id: requestId}));
    };
    signal?.addEventListener('abort', cancel, {once: true});
    ws.send(JSON.stringify({type: 'turn', request_id: requestId, turn}));
    try {
      for (;;) {
        const e = await q.next();
        if (e === null) return;
        yield e;
      }
    } finally {
      signal?.removeEventListener('abort', cancel);
      this.pending.delete(requestId);
    }
  }
}

const sockets = new Map<Surface, ChatSocket>();

/** The page's socket for a surface (one per tab and surface). */
export function chatSocket(surface: Surface): ChatSocket {
  let s = sockets.get(surface);
  if (!s) {
    s = new ChatSocket(surface);
    sockets.set(surface, s);
  }
  return s;
}

/**
 * One turn: over the socket when one can be opened, else over the HTTP event stream. Either way
 * the events are the same (chat.md §2) and come as they happen.
 */
export async function* chatTurn(surface: Surface, turn: TurnRequest, signal?: AbortSignal): AsyncGenerator<StreamEvent> {
  const socket = chatSocket(surface);
  if (socket.usable()) {
    let ws: WebSocket | null = null;
    try {
      ws = await socket.open();
    } catch {
      socket.failed();
    }
    if (ws) {
      yield* socket.turn(turn, ws, signal);
      return;
    }
  }
  socket.streaming();
  yield* sendTurn(surface, turn, signal);
}

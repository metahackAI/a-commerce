// Client for the chat stream (docs/api/chat.md §2): POST a turn, read SSE events as they arrive.
// Same-origin through the gateway (`/api/chat/...`), so cookies and CSRF work without CORS.

import {writeHeaders} from '@/lib/api';

import type {ElementContext, StreamEvent, Surface} from './types';

export interface TurnRequest {
  conversation_id?: string;
  message?: string;
  context?: ElementContext[];
  ui_event?: {block_id: string; action_id: string; args?: Record<string, unknown>};
}

/** Parses an SSE byte stream into events. Handles events split across chunks. */
export async function* readEvents(body: ReadableStream<Uint8Array>): AsyncGenerator<StreamEvent> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = '';
  for (;;) {
    const {value, done} = await reader.read();
    if (done) break;
    buffer += decoder.decode(value, {stream: true});
    let end: number;
    while ((end = buffer.indexOf('\n\n')) >= 0) {
      const raw = buffer.slice(0, end);
      buffer = buffer.slice(end + 2);
      const event: StreamEvent = {event: 'message', data: null};
      const data: string[] = [];
      for (const line of raw.split('\n')) {
        if (line.startsWith('event:')) event.event = line.slice(6).trim();
        else if (line.startsWith('id:')) event.id = line.slice(3).trim();
        else if (line.startsWith('data:')) data.push(line.slice(5).trimStart());
      }
      if (data.length > 0) {
        try {
          event.data = JSON.parse(data.join('\n'));
        } catch {
          event.data = data.join('\n');
        }
      }
      yield event;
    }
  }
}

/** Sends one turn and yields its events. Non-2xx answers become one `error` event. */
export async function* sendTurn(surface: Surface, turn: TurnRequest, signal?: AbortSignal): AsyncGenerator<StreamEvent> {
  const res = await fetch(`/api/chat/${surface}/turns`, {
    method: 'POST',
    headers: writeHeaders({accept: 'text/event-stream'}),
    credentials: 'same-origin',
    body: JSON.stringify(turn),
    signal,
  });
  if (!res.ok || !res.body) {
    let message = `The assistant is not available (${res.status}).`;
    try {
      const body = (await res.json()) as {error?: {message?: string}};
      if (body.error?.message) message = body.error.message;
    } catch {
      // Not JSON; keep the generic sentence.
    }
    yield {event: 'error', data: {code: 'upstream_unavailable', message}};
    return;
  }
  yield* readEvents(res.body);
}

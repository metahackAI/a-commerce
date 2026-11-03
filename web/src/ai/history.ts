// Earlier conversations (docs/api/chat.md §2): the caller's list on a surface, and one
// conversation's turns with their blocks, to show it again.

import {api} from '@/lib/api';

import type {Surface, UiBlock} from './types';

/** One line of the history list. Times are store time; `name` is the conversation's first request. */
export interface ConversationSummary {
  conversation_id: string;
  name: string | null;
  turns: number;
  started_at: string | null;
  last_turn_at: string | null;
}

/** One earlier turn: what the person said, the answer, and the blocks it sent. */
export interface PastTurn {
  turn_id: string;
  text: string;
  reply_text: string | null;
  blocks: UiBlock[] | null;
}

/** The caller's conversations on this surface, the most recent first. */
export function listConversations(surface: Surface, limit = 20): Promise<{items: ConversationSummary[]; total: number}> {
  return api(`/chat/${surface}/conversations`, {query: {limit}});
}

/** One of the caller's conversations; another person's answers `not_found`. */
export function loadConversation(surface: Surface, conversationId: string): Promise<{conversation_id: string; turns: PastTurn[]}> {
  return api(`/chat/${surface}/conversations/${encodeURIComponent(conversationId)}`);
}

const MONTHS = ['Jan', 'Feb', 'Mar', 'Apr', 'May', 'Jun', 'Jul', 'Aug', 'Sep', 'Oct', 'Nov', 'Dec'];

/**
 * `2026-11-06T10:52:00-05:00` → `Nov 6, 10:52`: when a conversation last moved, on the store's clock
 * as the server wrote it (the demo clock, so not compared with the viewer's own "today").
 */
export function whenText(stamp: string | null): string {
  const m = stamp ? /^(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2})/.exec(stamp) : null;
  return m ? `${MONTHS[Number(m[2]) - 1]} ${Number(m[3])}, ${m[4]}:${m[5]}` : '';
}

/** A conversation's name in a list: its first request, or a plain word when it has none. */
export function conversationName(c: ConversationSummary): string {
  return c.name?.trim() || 'Conversation';
}

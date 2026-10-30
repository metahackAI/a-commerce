'use client';

// `action_log` block (docs/api/chat.md §3.3): audit entries the agent looked up. Same rows as the
// activity page.

import type {ActionLogBlockData} from '@/app/bo/_lib/blocks';
import type {ActionEntry} from '@/app/bo/_lib/types';
import {ActionLogTable} from '@/components/bo/ActionLogTable';

import type {BlockProps} from '../types';
import {BlockFrame} from './parts';

export function ActionLogBlock({block}: BlockProps) {
  const d = block.data as ActionLogBlockData;
  const entries: ActionEntry[] = (d.entries ?? []).map((e, i) => ({
    id: e.id ?? `${block.block_id}_${i}`,
    '@type': e.action_type,
    action_status: e.status ?? 'completed',
    agent: typeof e.actor === 'object' && e.actor !== null ? (e.actor as ActionEntry['agent']) : {name: e.actor ? String(e.actor) : undefined},
    instrument: e.tool ?? null,
    object: typeof e.object === 'object' && e.object !== null ? (e.object as ActionEntry['object']) : e.object ? {id: String(e.object)} : null,
    result: typeof e.result === 'object' && e.result !== null ? (e.result as Record<string, unknown>) : e.result ? {text: e.result} : null,
    start_time: e.at ?? null,
  }));
  return (
    <BlockFrame block={block} kind="action_log" id={block.block_id} label="Action log">
      <ActionLogTable entries={entries} blockId={block.block_id} />
    </BlockFrame>
  );
}

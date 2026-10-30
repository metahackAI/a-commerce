'use client';

// memory_note: one line saying what Northwind remembered, with the shopper's own Undo (chat.md
// §3.3, customer.md preference). Undo is person-only (POST /v1/customer/me/memories/{id}/undo);
// the agent cannot press it.

import {Card} from '@astryxdesign/core/Card';
import {Icon} from '@astryxdesign/core/Icon';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {LightBulbIcon} from '@heroicons/react/24/outline';
import {useState} from 'react';

import type {BlockAction} from '@/ai/types';
import {useAiTarget} from '@/ai/useAiTarget';
import {Intents} from '@/components/fo/Ask';

import type {BlockProps} from '../types';
import {ActionButton, dataOf, intentsFor} from './common';
import type {MemoryNoteData} from './data';

export function MemoryNoteBlock({block}: BlockProps) {
  const data = dataOf<MemoryNoteData>(block);
  const [isUndone, setIsUndone] = useState(false);
  const memoryId = data.memory_id ?? block.block_id;
  const text = data.text ?? block.title ?? '';
  const target = useAiTarget({kind: 'memory_note', label: text, ids: {memory_id: memoryId}, term: 'acom:Preference', blockId: block.block_id});

  const own = block.actions.find(a => a.kind === 'person');
  const undo: BlockAction | null =
    own ??
    (data.memory_id
      ? {
          kind: 'person',
          action_id: 'undo_memory',
          label: 'Undo',
          method: 'POST',
          path: `/v1/customer/me/memories/${encodeURIComponent(data.memory_id)}/undo`,
          confirm: 'Forget this? Northwind stops using it.',
        }
      : null);
  const others = block.actions.filter(a => a !== own);

  return (
    <Card ref={target.ref} {...target.props} padding={2}>
      <VStack gap={1}>
        <HStack gap={2} vAlign="center" justify="between" wrap="wrap">
          <HStack gap={2} vAlign="center">
            <Icon icon={LightBulbIcon} size="sm" color="secondary" />
            <Text type="supporting" color={isUndone ? 'disabled' : 'secondary'} hasStrikethrough={isUndone}>
              {text}
            </Text>
          </HStack>
          {isUndone ? (
            <Text type="supporting" color="secondary">
              Forgotten
            </Text>
          ) : undo ? (
            <ActionButton action={undo} onDone={() => setIsUndone(true)} />
          ) : null}
        </HStack>
        {others.length > 0 ? (
          <HStack gap={1} wrap="wrap">
            {others.map(a => (
              <ActionButton key={a.action_id} action={a} />
            ))}
          </HStack>
        ) : null}
        <Intents says={intentsFor(block, 'memory_note', memoryId)} />
      </VStack>
    </Card>
  );
}

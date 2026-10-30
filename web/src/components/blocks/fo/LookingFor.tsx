'use client';

// looking_for: the conditions the assistant understood, as chips (chat.md §3.3). Removing a chip
// asks the assistant to search again without it; nothing changes until it answers.

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Token} from '@astryxdesign/core/Token';

import type {UiBlock} from '@/ai/types';
import {useAiTarget} from '@/ai/useAiTarget';
import {Intents, useSay} from '@/components/fo/Ask';

import type {BlockProps} from '../types';
import {BlockActions, dataOf, intentsFor, list} from './common';
import type {LookingForData} from './data';

const SOURCE: Record<string, string> = {profile: 'your profile', his: 'his', hers: 'hers', said: ''};

function Chip({block, chip}: {block: UiBlock; chip: LookingForData['chips'][number]}) {
  const say = useSay();
  const target = useAiTarget({kind: 'chip', label: chip.label, ids: {chip: chip.label}, blockId: block.block_id});
  const source = chip.source ? (SOURCE[chip.source] ?? chip.source) : '';
  const label = source ? `${chip.label} · ${source}` : chip.label;
  // A say button the block already carries for this chip wins over the generic sentence.
  const own = block.actions.find(
    a =>
      a.kind === 'say' &&
      (a.args?.chip === chip.label ||
        a.args?.remove === chip.label ||
        (/^(remove|drop)\b/i.test(a.label) && a.label.toLowerCase().includes(chip.label.toLowerCase()))),
  );
  return (
    <HStack gap={1} vAlign="center" ref={target.ref} {...target.props}>
      <Token
        label={label}
        size="sm"
        description={source ? `From ${source}` : undefined}
        onRemove={chip.removable === false ? undefined : () => say(own ? own.label : `Remove “${chip.label}”`)}
      />
      <Intents says={intentsFor(block, 'chip', chip.label)} />
    </HStack>
  );
}

export function LookingForBlock({block}: BlockProps) {
  const data = dataOf<LookingForData>(block);
  const chips = list(data.chips);
  const target = useAiTarget({kind: 'block', label: block.title ?? 'Looking for', ids: {block_id: block.block_id}, blockId: block.block_id});
  return (
    <VStack gap={2} ref={target.ref} {...target.props}>
      <HStack gap={2} vAlign="center" wrap="wrap">
        <Text type="supporting" color="secondary">
          {block.title ?? 'Looking for'}
        </Text>
        {chips.map(c => (
          <Chip key={c.label} block={block} chip={c} />
        ))}
      </HStack>
      {block.why ? (
        <Text type="supporting" color="secondary">
          {block.why}
        </Text>
      ) : null}
      <BlockActions block={block} only={['open', 'person']} />
    </VStack>
  );
}

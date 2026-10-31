'use client';

// profile_card: "What Ailism knows about you" (chat.md §3.3, get_my_profile). Each fact is an
// AI target so the shopper can ask why it is there; changes go through the block's buttons
// (`say` to correct, `person` to delete).

import {HStack, VStack} from '@astryxdesign/core/Layout';
import {List, ListItem} from '@astryxdesign/core/List';
import {Text} from '@astryxdesign/core/Text';

import type {UiBlock} from '@/ai/types';
import {useAiTarget} from '@/ai/useAiTarget';
import {words} from '@/app/(storefront)/_lib/format';
import {AskButton, Intents} from '@/components/fo/Ask';

import type {BlockProps} from '../types';
import {ActionButton, BlockActions, BlockFrame, dataOf, intentsFor, list} from './common';
import type {ProfileCardData} from './data';
import {textOf} from './values';

type Fact = ProfileCardData['facts'][number];

const SOURCE: Record<string, string> = {conversation: 'From a conversation', profile: 'You set it', orders: 'From your orders'};

function FactRow({block, fact}: {block: UiBlock; fact: Fact}) {
  const value = textOf(fact.value);
  const id = fact.memory_id ?? fact.key;
  const target = useAiTarget({
    kind: 'profile_fact',
    label: `${words(fact.key)}: ${value}`,
    ids: fact.memory_id ? {memory_id: fact.memory_id} : {key: fact.key},
    term: 'acom:Preference',
    blockId: block.block_id,
  });
  // Buttons the block carries for this fact (args.memory_id or args.key), e.g. a person-only delete.
  const own = block.actions.filter(
    a => (a.kind === 'say' && (a.args?.memory_id === id || a.args?.key === fact.key)) || (a.kind === 'person' && fact.memory_id && a.path.includes(fact.memory_id)),
  );
  return (
    <ListItem
      ref={target.ref}
      {...target.props}
      align="start"
      label={<Text type="body">{value}</Text>}
      description={
        <VStack gap={0.5}>
          <Text type="supporting" color="secondary">
            {[words(fact.key), fact.source ? (SOURCE[fact.source] ?? words(fact.source)) : null].filter(Boolean).join(' · ')}
          </Text>
          <Intents says={intentsFor(block, 'profile_fact', id)} />
        </VStack>
      }
      endContent={
        <HStack gap={1} vAlign="center">
          {own.map(a => (
            <ActionButton blockId={block.block_id} key={a.action_id} action={a} />
          ))}
          <AskButton target={target} label={value} />
        </HStack>
      }
    />
  );
}

export function ProfileCardBlock({block}: BlockProps) {
  const facts = list(dataOf<ProfileCardData>(block).facts);
  const perFact = new Set(
    block.actions
      .filter(a =>
        facts.some(
          f =>
            (a.kind === 'say' && (a.args?.memory_id === (f.memory_id ?? f.key) || a.args?.key === f.key)) ||
            (a.kind === 'person' && f.memory_id && a.path.includes(f.memory_id)),
        ),
      )
      .map(a => a.action_id),
  );
  const rest = {...block, actions: block.actions.filter(a => !perFact.has(a.action_id))};
  return (
    <BlockFrame block={{...block, title: block.title ?? 'What Ailism knows about you'}} footer={<BlockActions block={rest} />}>
      {facts.length === 0 ? (
        <Text type="supporting" color="secondary">
          Nothing remembered yet.
        </Text>
      ) : (
        <List hasDividers edge="flush" density="compact">
          {facts.map(f => (
            <FactRow key={`${f.key}-${f.memory_id ?? textOf(f.value)}`} block={block} fact={f} />
          ))}
        </List>
      )}
    </BlockFrame>
  );
}

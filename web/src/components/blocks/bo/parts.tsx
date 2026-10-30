'use client';

// What every back-office block shares: its frame (title, why-line, Ask), its buttons
// (docs/api/chat.md §3.2) and the lookup of suggested intents per element.
//
// - `person` buttons render as PersonButton only, and only when the session holds the scope the
//   endpoint needs. A reject asks for the reason first.
// - `say` buttons send their label to the agent.
// - `open` buttons go to a page, or scroll to another block on screen.

import {useState, type ReactNode} from 'react';
import {useRouter} from 'next/navigation';
import {HStack, StackItem, VStack} from '@astryxdesign/core/Layout';
import {Card} from '@astryxdesign/core/Card';
import {Heading, Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';
import {TextArea} from '@astryxdesign/core/TextArea';

import {useAi} from '@/ai/AiProvider';
import {useAiTarget} from '@/ai/useAiTarget';
import type {BlockAction, UiBlock} from '@/ai/types';
import {PersonButton} from '@/components/PersonButton';
import {useAgentDock} from '@/components/bo/AgentDock';
import {mayPress, useBoSession} from '@/components/bo/BoSession';
import {AskButton} from '@/components/bo/targets';
import type {IntentsFor} from '@/components/bo/ChangeParts';

/** Suggested intents of a block for one element (`<kind>:<id>`). */
export function intentsLookup(block: UiBlock): IntentsFor {
  const map = new Map<string, string[]>();
  for (const s of block.suggested_intents ?? []) {
    const list = map.get(s.element) ?? [];
    list.push(s.say);
    map.set(s.element, list);
  }
  return (kind, id) => map.get(`${kind}:${id}`);
}

/** Intents for the block as a whole (`element` is the block id or `block`). */
function blockIntents(block: UiBlock): string[] {
  return (block.suggested_intents ?? []).filter(s => s.element === block.block_id || s.element === 'block').map(s => s.say);
}

/** The block's frame: title, why-line, Ask about the whole block, the content, the buttons. */
export function BlockFrame({
  block,
  kind,
  id,
  label,
  meta,
  children,
  hideActions,
}: {
  block: UiBlock;
  kind: string;
  id: string;
  label?: string;
  meta?: ReactNode;
  children: ReactNode;
  hideActions?: (a: BlockAction) => boolean;
}) {
  const ai = useAi();
  const dock = useAgentDock();
  const title = block.title ?? label ?? block.ui;
  const t = useAiTarget({kind, label: label ?? title, ids: {[`${kind}_id`]: id, block_id: block.block_id}, blockId: block.block_id});
  const intents = blockIntents(block);
  return (
    <Card ref={t.ref} {...t.props}>
      <VStack gap={4}>
        <HStack gap={2} vAlign="start">
          <StackItem size="fill">
            <VStack gap={1}>
              <Heading level={3}>{title}</Heading>
              {meta ? (
                <HStack gap={2} vAlign="center" wrap="wrap">
                  {meta}
                </HStack>
              ) : null}
              {block.why ? (
                <Text type="supporting" color="secondary">
                  {block.why}
                </Text>
              ) : null}
            </VStack>
          </StackItem>
          <AskButton target={t} label={title} />
        </HStack>
        {children}
        {intents.length > 0 ? (
          <HStack gap={1} wrap="wrap">
            {intents.map(say => (
              <Button
                key={say}
                label={say}
                size="sm"
                variant="ghost"
                onClick={() => {
                  dock.open();
                  void ai.send(say);
                }}
              />
            ))}
          </HStack>
        ) : null}
        <BlockActions block={block} hide={hideActions} />
      </VStack>
    </Card>
  );
}

function RejectWithReason({action}: {action: Extract<BlockAction, {kind: 'person'}>}) {
  const router = useRouter();
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState('');
  if (!open) return <Button label={action.label} onClick={() => setOpen(true)} />;
  const base = action.body && typeof action.body === 'object' ? (action.body as Record<string, unknown>) : {};
  return (
    <VStack gap={2} width={360}>
      <TextArea label="Why reject it?" value={reason} onChange={setReason} maxLength={500} rows={2} hasAutoFocus />
      <HStack gap={2} hAlign="end" vAlign="center">
        <Button label="Cancel" variant="ghost" onClick={() => setOpen(false)} />
        {reason.trim() ? (
          <PersonButton
            action={{...action, body: {...base, reason: reason.trim()}, confirm: `${action.confirm} Reason: “${reason.trim()}”`}}
            onDone={() => {
              setOpen(false);
              router.refresh();
            }}
          />
        ) : (
          <Text type="supporting" color="secondary">
            Write a reason to reject.
          </Text>
        )}
      </HStack>
    </VStack>
  );
}

/** A block's buttons by kind (docs/api/chat.md §3.2). */
export function BlockActions({block, hide}: {block: UiBlock; hide?: (a: BlockAction) => boolean}) {
  const ai = useAi();
  const dock = useAgentDock();
  const router = useRouter();
  const session = useBoSession();
  const actions = block.actions.filter(a => !hide?.(a));
  if (actions.length === 0) return null;

  return (
    <HStack gap={2} hAlign="end" vAlign="start" wrap="wrap">
      {actions.map(a => {
        switch (a.kind) {
          case 'person':
            if (!mayPress(session, a.method, a.path)) return null;
            if (/\/reject$/.test(a.path)) return <RejectWithReason key={a.action_id} action={a} />;
            return <PersonButton key={a.action_id} action={a} onDone={() => router.refresh()} />;
          case 'say':
            return (
              <Button
                key={a.action_id}
                label={a.label}
                onClick={() => {
                  dock.open();
                  void ai.send(a.label);
                }}
              />
            );
          case 'open':
            return (
              <Button
                key={a.action_id}
                label={a.label}
                variant="ghost"
                onClick={() => {
                  if (a.target.startsWith('block:')) {
                    const id = a.target.slice('block:'.length);
                    document.querySelector(`[data-acom-ref^="${CSS.escape(id)}#"]`)?.scrollIntoView({behavior: 'smooth', block: 'start'});
                  } else {
                    router.push(a.target);
                  }
                }}
              />
            );
          default:
            return null;
        }
      })}
    </HStack>
  );
}

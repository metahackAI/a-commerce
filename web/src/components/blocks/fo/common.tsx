'use client';

// Parts every storefront block shares (docs/api/chat.md §3): the frame with title and why-line,
// the block's buttons, and the suggested sentences attached to one element.
//
// Buttons: `person` renders as PersonButton (the only path to a person-only endpoint), `say`
// sends its sentence to the assistant, `open` navigates or scrolls to another block.

import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {HStack, VStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {useRouter} from 'next/navigation';
import type {ReactNode} from 'react';

import type {BlockAction, UiBlock} from '@/ai/types';
import {useAiTarget} from '@/ai/useAiTarget';
import {PersonButton} from '@/components/PersonButton';
import {useSay} from '@/components/fo/Ask';

/** The block's suggested sentences for one element (`<kind>:<id>`). */
export function intentsFor(block: UiBlock, kind: string, id: string): string[] {
  const element = `${kind}:${id}`;
  return (block.suggested_intents ?? []).filter(i => i.element === element).map(i => i.say);
}

/** The block's data as an object, whatever arrived. */
export function dataOf<T>(block: UiBlock): Partial<T> {
  return block.data && typeof block.data === 'object' ? (block.data as Partial<T>) : {};
}

/** A list field that may be missing. */
export function list<T>(v: T[] | null | undefined): T[] {
  return Array.isArray(v) ? v : [];
}

export function ActionButton({action, onDone}: {action: BlockAction; onDone?: (result: unknown) => void}) {
  const say = useSay();
  const router = useRouter();
  if (action.kind === 'person') {
    return (
      <PersonButton
        action={{label: action.label, method: action.method, path: action.path, body: action.body, confirm: action.confirm}}
        onDone={result => {
          onDone?.(result);
          router.refresh();
        }}
      />
    );
  }
  if (action.kind === 'say') {
    return <Button label={action.label} size="sm" onClick={() => say(action.label)} />;
  }
  return (
    <Button
      label={action.label}
      size="sm"
      variant="ghost"
      onClick={() => {
        if (action.target.startsWith('block:')) {
          const id = action.target.slice('block:'.length);
          document.querySelector(`[data-acom-ref^="${CSS.escape(id)}#"]`)?.scrollIntoView({behavior: 'smooth', block: 'start'});
        } else {
          router.push(action.target);
        }
      }}
    />
  );
}

/** The block's buttons in one row; `only` picks some by kind. */
export function BlockActions({
  block,
  only,
  onDone,
}: {
  block: UiBlock;
  only?: BlockAction['kind'][];
  onDone?: (result: unknown) => void;
}) {
  const actions = block.actions.filter(a => !only || only.includes(a.kind));
  if (actions.length === 0) return null;
  return (
    <HStack gap={2} wrap="wrap" vAlign="center">
      {actions.map(a => (
        <ActionButton key={a.action_id} action={a} onDone={onDone} />
      ))}
    </HStack>
  );
}

/** Title, why-line and buttons around a block's own content. The frame is the block's AI target. */
export function BlockFrame({
  block,
  children,
  footer,
  hideActions,
  aside,
}: {
  block: UiBlock;
  children: ReactNode;
  /** Replaces the default action row. */
  footer?: ReactNode;
  hideActions?: boolean;
  /** Small text beside the title (counts, sort order). */
  aside?: ReactNode;
}) {
  const target = useAiTarget({
    kind: 'block',
    label: block.title ?? block.ui,
    ids: {block_id: block.block_id},
    blockId: block.block_id,
  });
  return (
    <Card ref={target.ref} {...target.props}>
      <VStack gap={3}>
        {block.title || aside ? (
          <HStack gap={2} vAlign="center" wrap="wrap">
            {block.title ? (
              <Text type="label" weight="semibold">
                {block.title}
              </Text>
            ) : null}
            {aside ? (
              <Text type="supporting" color="secondary">
                {aside}
              </Text>
            ) : null}
          </HStack>
        ) : null}
        {children}
        {block.why ? (
          <Text type="supporting" color="secondary">
            {block.why}
          </Text>
        ) : null}
        {footer ?? (hideActions ? null : <BlockActions block={block} />)}
      </VStack>
    </Card>
  );
}

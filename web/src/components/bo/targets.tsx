'use client';

// Elements people can ask about (docs/frontend/README.md §3): each wrapper registers its element
// with `useAiTarget` (so the agent can point at it with `ui.command`) and carries an Ask button
// that picks it as "this" for the next message. Suggested intents, when a block provides them,
// sit next to the element and send their sentence as is.

import {useEffect, useRef, type ReactNode} from 'react';
import {HStack, StackItem, VStack} from '@astryxdesign/core/Layout';
import {Button} from '@astryxdesign/core/Button';
import {IconButton} from '@astryxdesign/core/IconButton';
import {Icon} from '@astryxdesign/core/Icon';
import {Card} from '@astryxdesign/core/Card';
import {MetadataListItem} from '@astryxdesign/core/MetadataList';
import {TableCell, TableRow} from '@astryxdesign/core/Table';
import {ChatBubbleOvalLeftEllipsisIcon} from '@heroicons/react/24/outline';

import {useAi} from '@/ai/AiProvider';
import {useAiTarget, type AiTarget} from '@/ai/useAiTarget';

import {useAgentDock} from './AgentDock';

type Registered = ReturnType<typeof useAiTarget>;

/** Picks the element as "this" and opens the conversation. */
export function AskButton({target, label}: {target: Registered; label: string}) {
  const dock = useAgentDock();
  return (
    <IconButton
      label={`Ask about ${label}`}
      tooltip="Ask the agent about this"
      variant="ghost"
      size="sm"
      icon={<Icon icon={ChatBubbleOvalLeftEllipsisIcon} />}
      onClick={() => {
        target.ask();
        dock.open();
      }}
    />
  );
}

/**
 * Sends a sentence with an element picked as "this". The pick is state in the AI layer, so the
 * sentence goes after the pick has rendered, with the `send` of that render.
 */
export function useSendAbout() {
  const ai = useAi();
  const latest = useRef(ai.send);
  useEffect(() => {
    latest.current = ai.send;
  });
  return (say: string, target?: Registered) => {
    if (!target) {
      void ai.send(say);
      return;
    }
    target.ask();
    window.setTimeout(() => void latest.current(say), 0);
  };
}

/** Suggested sentences for an element; pressing one sends it to the agent with the element picked. */
export function SuggestedIntents({intents, target}: {intents?: string[]; target?: Registered}) {
  const sendAbout = useSendAbout();
  const dock = useAgentDock();
  if (!intents || intents.length === 0) return null;
  return (
    <HStack gap={1} wrap="wrap">
      {intents.map(say => (
        <Button
          key={say}
          label={say}
          size="sm"
          variant="ghost"
          onClick={() => {
            dock.open();
            sendAbout(say, target);
          }}
        />
      ))}
    </HStack>
  );
}

/** A table row that is an AI target. Its last cell holds Ask, so the header needs one more cell. */
export function TargetRow({target, intents, children}: {target: AiTarget; intents?: string[]; children: ReactNode}) {
  const t = useAiTarget(target);
  return (
    <TableRow ref={t.ref} {...t.props}>
      {children}
      <TableCell>
        <HStack gap={1} vAlign="center" hAlign="end">
          <SuggestedIntents intents={intents} target={t} />
          <AskButton target={t} label={target.label} />
        </HStack>
      </TableCell>
    </TableRow>
  );
}

/** A card that is an AI target, with the Ask button in its top row. */
export function TargetCard({
  target,
  intents,
  heading,
  children,
  variant,
}: {
  target: AiTarget;
  intents?: string[];
  heading?: ReactNode;
  children?: ReactNode;
  variant?: 'default' | 'muted' | 'transparent';
}) {
  const t = useAiTarget(target);
  return (
    <Card ref={t.ref} {...t.props} variant={variant}>
      <VStack gap={2}>
        <HStack gap={2} vAlign="start">
          <StackItem size="fill">
            <VStack gap={1}>{heading}</VStack>
          </StackItem>
          <AskButton target={t} label={target.label} />
        </HStack>
        {children}
        <SuggestedIntents intents={intents} target={t} />
      </VStack>
    </Card>
  );
}

/** Any inline content (a price, a figure, a chip) that is an AI target. */
export function TargetInline({target, children, intents}: {target: AiTarget; children: ReactNode; intents?: string[]}) {
  const t = useAiTarget(target);
  return (
    <HStack ref={t.ref} {...t.props} gap={1} vAlign="center" wrap="wrap">
      {children}
      <AskButton target={t} label={target.label} />
      <SuggestedIntents intents={intents} target={t} />
    </HStack>
  );
}

/** A labelled fact (MetadataListItem) that is an AI target. */
export function TargetFact({target, label, children, intents}: {target: AiTarget; label: string; children: ReactNode; intents?: string[]}) {
  const t = useAiTarget(target);
  return (
    <MetadataListItem ref={t.ref} {...t.props} label={label}>
      <VStack gap={1}>
        <HStack gap={1} vAlign="center" wrap="wrap">
          {children}
          <AskButton target={t} label={target.label} />
        </HStack>
        <SuggestedIntents intents={intents} target={t} />
      </VStack>
    </MetadataListItem>
  );
}

/** A form field that is an AI target: the input, and Ask beside it at the field's baseline. */
export function TargetField({target, children}: {target: AiTarget; children: ReactNode}) {
  const t = useAiTarget(target);
  return (
    <HStack ref={t.ref} {...t.props} gap={1} vAlign="end" wrap="wrap">
      {children}
      <AskButton target={t} label={target.label} />
    </HStack>
  );
}

/** Registers a region the agent can point at, without an Ask button (filters, toolbars). */
export function TargetRegion({target, children}: {target: AiTarget; children: ReactNode}) {
  const t = useAiTarget(target);
  return (
    <VStack ref={t.ref} {...t.props} gap={0}>
      {children}
    </VStack>
  );
}

/** A stacked block of content that is an AI target (a section, a group). */
export function TargetBlock({target, children, intents}: {target: AiTarget; children: ReactNode; intents?: string[]}) {
  const t = useAiTarget(target);
  return (
    <VStack ref={t.ref} {...t.props} gap={2}>
      <HStack gap={2} vAlign="start">
        <StackItem size="fill">
          <VStack gap={2}>{children}</VStack>
        </StackItem>
        <AskButton target={t} label={target.label} />
      </HStack>
      <SuggestedIntents intents={intents} target={t} />
    </VStack>
  );
}

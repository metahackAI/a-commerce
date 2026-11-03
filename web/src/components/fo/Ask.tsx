'use client';

// The two ways a shopper talks about what is on screen (docs/frontend/README.md §3.2): pick an
// element as "this" and ask, or press one of the suggested sentences the agent attached to it.
// Both open the assistant. Neither can change state on its own; person decisions stay on
// PersonButton.

import {HStack} from '@astryxdesign/core/Layout';
import {Button} from '@astryxdesign/core/Button';
import {Icon} from '@astryxdesign/core/Icon';
import {IconButton} from '@astryxdesign/core/IconButton';
import {ChatBubbleLeftEllipsisIcon} from '@heroicons/react/24/outline';
import {useCallback} from 'react';

import {useAi} from '@/ai/AiProvider';
import type {ElementContext} from '@/ai/types';

import {useStorefrontIfAny} from './StorefrontContext';

const stayOpen = () => {};

/**
 * Opens the storefront's assistant. A storefront block shown in the back office (an order on the
 * owner's conversation) is already in an open conversation, so there is nothing to open.
 */
function useOpenAssistant(): () => void {
  return useStorefrontIfAny()?.openAssistant ?? stayOpen;
}

/** Sends a sentence to the assistant and opens it; `about` is the element the sentence is about. */
export function useSay() {
  const ai = useAi();
  const openAssistant = useOpenAssistant();
  return useCallback(
    (text: string, about?: ElementContext) => {
      openAssistant();
      void ai.send(text, undefined, about ? [about] : []);
    },
    [ai, openAssistant],
  );
}

/** A block's `say` button: sends the block's intent (`ui_event`) and opens the assistant. */
export function useSayEvent() {
  const ai = useAi();
  const openAssistant = useOpenAssistant();
  return useCallback(
    (blockId: string, actionId: string, label: string, args?: Record<string, unknown>) => {
      openAssistant();
      void ai.sendEvent(blockId, actionId, label, args);
    },
    [ai, openAssistant],
  );
}

/** "Ask about this": picks the element as context for the next message and opens the assistant. */
export function AskButton({target, label}: {target: {ask: () => void}; label: string}) {
  const openAssistant = useOpenAssistant();
  return (
    <IconButton
      label={`Ask about ${label}`}
      tooltip="Ask about this"
      variant="ghost"
      size="sm"
      icon={<Icon icon={ChatBubbleLeftEllipsisIcon} />}
      onClick={() => {
        target.ask();
        openAssistant();
      }}
    />
  );
}

/**
 * Suggested sentences for one element (a block's `suggested_intents`); pressing one sends it with
 * the element as its context, so "this one" reaches the agent as that element.
 */
export function Intents({says, about}: {says: string[]; about: ElementContext}) {
  const say = useSay();
  if (says.length === 0) return null;
  return (
    <HStack gap={1} wrap="wrap">
      {says.map(s => (
        <Button key={s} label={s} size="sm" variant="ghost" onClick={() => say(s, about)} />
      ))}
    </HStack>
  );
}

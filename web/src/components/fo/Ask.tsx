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

import {useStorefront} from './StorefrontContext';

/** Sends a sentence to the assistant and opens it. */
export function useSay() {
  const ai = useAi();
  const {openAssistant} = useStorefront();
  return useCallback(
    (text: string) => {
      openAssistant();
      void ai.send(text);
    },
    [ai, openAssistant],
  );
}

/** "Ask about this": picks the element as context for the next message and opens the assistant. */
export function AskButton({target, label}: {target: {ask: () => void}; label: string}) {
  const {openAssistant} = useStorefront();
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

/** Suggested sentences for one element (a block's `suggested_intents`); pressing one sends it. */
export function Intents({says}: {says: string[]}) {
  const say = useSay();
  if (says.length === 0) return null;
  return (
    <HStack gap={1} wrap="wrap">
      {says.map(s => (
        <Button key={s} label={s} size="sm" variant="ghost" onClick={() => say(s)} />
      ))}
    </HStack>
  );
}

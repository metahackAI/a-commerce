'use client';

// The agent beside every back-office page. "Ask" on any element picks it as "this" and opens the
// dock, where the person types the question; the agent's reply and blocks show here too. The
// home page shows the full workspace instead and takes over `open()` while it is mounted.
// Person decisions are never made from here except through the blocks' PersonButtons.

import {createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode} from 'react';
import {useRouter} from 'next/navigation';
import {VStack, HStack, StackItem} from '@astryxdesign/core/Layout';
import {Drawer, DrawerHeader} from '@astryxdesign/core/Drawer';
import {
  ChatComposer,
  ChatLayout,
  ChatMessage,
  ChatMessageBubble,
  ChatMessageList,
  ChatSystemMessage,
} from '@astryxdesign/core/Chat';
import {EmptyState} from '@astryxdesign/core/EmptyState';
import {Button} from '@astryxdesign/core/Button';
import {Token} from '@astryxdesign/core/Token';

import {useAi} from '@/ai/AiProvider';

import {BlockView} from './BlockView';

interface DockState {
  isOpen: boolean;
  /** Shows the conversation: the dock, or the page's own workspace when one is mounted. */
  open: () => void;
  close: () => void;
  /** A page with its own workspace routes `open()` to it while mounted; returns the reset. */
  takeOver: (opener: () => void) => () => void;
}

const DockContext = createContext<DockState | null>(null);

export function AgentDockProvider({children}: {children: ReactNode}) {
  const [isOpen, setIsOpen] = useState(false);
  const opener = useRef<(() => void) | null>(null);

  const open = useCallback(() => {
    if (opener.current) opener.current();
    else setIsOpen(true);
  }, []);
  const close = useCallback(() => setIsOpen(false), []);
  const takeOver = useCallback((fn: () => void) => {
    opener.current = fn;
    setIsOpen(false);
    return () => {
      if (opener.current === fn) opener.current = null;
    };
  }, []);

  const value = useMemo(() => ({isOpen, open, close, takeOver}), [isOpen, open, close, takeOver]);
  return (
    <DockContext.Provider value={value}>
      {children}
      <AgentDrawer isOpen={isOpen} onOpenChange={setIsOpen} />
    </DockContext.Provider>
  );
}

export function useAgentDock(): DockState {
  const ctx = useContext(DockContext);
  if (!ctx) throw new Error('useAgentDock must be used inside <AgentDockProvider>');
  return ctx;
}

/** A composer that sends to the agent, with the picked elements as removable chips. */
export function AgentComposer({placeholder, onSent}: {placeholder: string; onSent?: () => void}) {
  const ai = useAi();
  return (
    <ChatComposer
      onSubmit={text => {
        void ai.send(text);
        onSent?.();
      }}
      placeholder={placeholder}
      isDisabled={ai.isBusy}
      status={ai.isBusy ? {type: 'warning', message: 'Working…'} : undefined}
      headerActions={
        ai.picked.length > 0 ? (
          <HStack gap={1} wrap="wrap">
            {ai.picked.map(p => (
              <Token key={p.element_ref} label={p.label} size="sm" onRemove={() => ai.unpick(p.element_ref)} />
            ))}
          </HStack>
        ) : undefined
      }
    />
  );
}

function AgentDrawer({isOpen, onOpenChange}: {isOpen: boolean; onOpenChange: (open: boolean) => void}) {
  const ai = useAi();
  const router = useRouter();

  return (
    <Drawer isOpen={isOpen} onOpenChange={onOpenChange} label="Ask the agent" width={480} hasScrim={false} padding={0}>
      <VStack height="100%">
        <DrawerHeader
          title="Ask the agent"
          subtitle="Press Ask on anything on the page, then say what you want to know or change."
          onOpenChange={onOpenChange}
          hasDivider
          endContent={
            <Button
              label="Full view"
              variant="ghost"
              size="sm"
              onClick={() => {
                onOpenChange(false);
                router.push('/bo?view=agent');
              }}
            />
          }
        />
        <StackItem size="fill">
          <ChatLayout density="compact" composer={<AgentComposer placeholder="Ask about this, or ask for a change…" />}>
            <ChatMessageList
              density="compact"
              isStreaming={ai.isBusy}
              emptyState={
                <EmptyState
                  isCompact
                  title="Nothing asked yet"
                  description="The agent can explain anything here and prepare changes. Nothing changes in the store until a person approves."
                />
              }>
              {ai.messages.map(m =>
                m.sender === 'system' ? (
                  <ChatSystemMessage key={m.id}>{m.text}</ChatSystemMessage>
                ) : (
                  <ChatMessage key={m.id} sender={m.sender}>
                    <ChatMessageBubble>{m.text}</ChatMessageBubble>
                  </ChatMessage>
                ),
              )}
              {ai.blocks.length > 0 ? <ChatSystemMessage variant="divider">Results</ChatSystemMessage> : null}
              {ai.blocks.map(b => (
                <ChatMessage key={b.block_id} sender="assistant">
                  <BlockView block={b} />
                </ChatMessage>
              ))}
            </ChatMessageList>
          </ChatLayout>
        </StackItem>
      </VStack>
    </Drawer>
  );
}

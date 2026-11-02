'use client';

// The agent beside every back-office page. "Ask" on any element picks it as "this" and opens the
// dock, where the person types the question; the agent's reply and blocks show here too. The
// home page shows the full workspace instead and takes over `open()` while it is mounted.
// Person decisions are never made from here except through the blocks' PersonButtons.

import {createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode} from 'react';
import {useRouter} from 'next/navigation';
import {Drawer} from '@astryxdesign/core/Drawer';
import {ChatComposer} from '@astryxdesign/core/Chat';
import {Button} from '@astryxdesign/core/Button';
import {Icon} from '@astryxdesign/core/Icon';
import {IconButton} from '@astryxdesign/core/IconButton';
import {XMarkIcon} from '@heroicons/react/24/outline';

import {useAi} from '@/ai/AiProvider';

import {ChatEmpty, ChatHeader, ChatPanel, PickedChips} from '@/components/chat/ChatParts';
import {ConversationActions} from '@/components/chat/History';
import {useVoiceInput} from '@/components/VoiceInput';

const DOCK_STARTERS = ['What needs my attention today?', 'Why did sales drop yesterday?', 'Which products are low on stock?'];

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

// Outside the back office (a block rendered in another surface's workspace) the conversation is
// already on screen, so there is nothing to open.
const NO_DOCK: DockState = {isOpen: false, open: () => {}, close: () => {}, takeOver: () => () => {}};

export function useAgentDock(): DockState {
  return useContext(DockContext) ?? NO_DOCK;
}

/** A composer that sends to the agent, with the picked elements as removable chips. */
export function AgentComposer({placeholder, onSent}: {placeholder: string; onSent?: () => void}) {
  const ai = useAi();
  const voice = useVoiceInput();
  return (
    <ChatComposer
      onSubmit={text => {
        void ai.send(text);
        onSent?.();
      }}
      placeholder={placeholder}
      elevation="none"
      isStopShown={ai.isBusy}
      onStop={ai.stop}
      sendActions={voice.button}
      status={ai.isBusy ? {type: 'warning', message: 'Working…'} : voice.status}
      headerActions={ai.picked.length > 0 ? <PickedChips /> : undefined}
    />
  );
}

function AgentDrawer({isOpen, onOpenChange}: {isOpen: boolean; onOpenChange: (open: boolean) => void}) {
  const router = useRouter();
  return (
    <Drawer isOpen={isOpen} onOpenChange={onOpenChange} label="Ask the agent" width={480} hasScrim={false} padding={0}>
      {isOpen ? (
        <ChatPanel
          header={
            <ChatHeader
              subtitle="Press Ask on anything, then say what to know or change."
              actions={
                <>
                  <ConversationActions />
                  <Button
                    label="Full view"
                    variant="ghost"
                    size="sm"
                    onClick={() => {
                      onOpenChange(false);
                      router.push('/bo?view=agent');
                    }}
                  />
                  <IconButton label="Close" variant="ghost" size="sm" icon={<Icon icon={XMarkIcon} />} onClick={() => onOpenChange(false)} />
                </>
              }
            />
          }
          placeholder="Ask about this, or ask for a change…"
          emptyState={
            <ChatEmpty
              title="Nothing asked yet"
              description="The agent explains anything here and prepares changes. Nothing changes in the store until a person approves."
              starters={DOCK_STARTERS}
            />
          }
        />
      ) : null}
    </Drawer>
  );
}

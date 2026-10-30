'use client';

// The AI layer (docs/frontend/README.md §3): conversation state, turn sending, the registry of
// elements people can ask about, and the screen commands the agent may run (highlight, scroll).
// Person-only actions are not here: they go through PersonButton and never through the agent.

import {createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode} from 'react';

import {sendTurn} from './stream';
import type {ChatMessage, ElementContext, Surface, UiBlock} from './types';

interface AiState {
  surface: Surface;
  messages: ChatMessage[];
  blocks: UiBlock[];
  isBusy: boolean;
  /** Elements the person picked as "this" for the next message. */
  picked: ElementContext[];
  send: (text: string) => Promise<void>;
  pick: (element: ElementContext) => void;
  unpick: (elementRef: string) => void;
  /** Registered elements by element_ref, for `ui.command` targets. */
  targets: Map<string, HTMLElement>;
}

const AiContext = createContext<AiState | null>(null);

let counter = 0;
const nextId = (prefix: string) => `${prefix}_${Date.now().toString(36)}${(counter++).toString(36)}`;

export function AiProvider({surface, children}: {surface: Surface; children: ReactNode}) {
  const [messages, setMessages] = useState<ChatMessage[]>([]);
  const [blocks, setBlocks] = useState<UiBlock[]>([]);
  const [isBusy, setIsBusy] = useState(false);
  const [picked, setPicked] = useState<ElementContext[]>([]);
  const conversationId = useRef<string | undefined>(undefined);
  const targets = useRef(new Map<string, HTMLElement>()).current;

  const runCommand = useCallback(
    (command: {command?: string; target?: string}) => {
      const el = command.target ? targets.get(command.target) : undefined;
      if (!el) return; // only registered targets; PersonButton never registers
      if (command.command === 'scroll_to' || command.command === 'highlight') {
        el.scrollIntoView({behavior: 'smooth', block: 'center'});
      }
      if (command.command === 'highlight') {
        el.dataset.acomHighlight = 'true';
        window.setTimeout(() => delete el.dataset.acomHighlight, 4000);
      }
    },
    [targets],
  );

  const send = useCallback(
    async (text: string) => {
      const context = picked;
      setMessages(m => [...m, {id: nextId('msg'), sender: 'user', text, context}]);
      setPicked([]);
      setIsBusy(true);
      let reply = '';
      const replyId = nextId('msg');
      try {
        for await (const ev of sendTurn(surface, {conversation_id: conversationId.current, message: text, context})) {
          const data = ev.data as Record<string, unknown>;
          if (ev.event === 'turn.started' && typeof data.conversation_id === 'string') {
            conversationId.current = data.conversation_id;
          } else if (ev.event === 'message.delta' && typeof data.text === 'string') {
            reply += data.text;
            const text = reply;
            setMessages(m => [...m.filter(x => x.id !== replyId), {id: replyId, sender: 'assistant', text}]);
          } else if (ev.event === 'ui.block') {
            const block = ev.data as UiBlock;
            setBlocks(b => [...b.filter(x => x.block_id !== block.block_id), block]);
          } else if (ev.event === 'ui.command') {
            runCommand(data as {command?: string; target?: string});
          } else if (ev.event === 'error') {
            const message = typeof data.message === 'string' ? data.message : 'Something went wrong.';
            setMessages(m => [...m, {id: nextId('msg'), sender: 'system', text: message}]);
          }
        }
      } catch {
        setMessages(m => [...m, {id: nextId('msg'), sender: 'system', text: 'The assistant could not be reached.'}]);
      } finally {
        setIsBusy(false);
      }
    },
    [picked, runCommand, surface],
  );

  const pick = useCallback((element: ElementContext) => {
    setPicked(p => (p.some(x => x.element_ref === element.element_ref) ? p : [...p, element]));
  }, []);
  const unpick = useCallback((ref: string) => setPicked(p => p.filter(x => x.element_ref !== ref)), []);

  const value = useMemo(
    () => ({surface, messages, blocks, isBusy, picked, send, pick, unpick, targets}),
    [surface, messages, blocks, isBusy, picked, send, pick, unpick, targets],
  );
  return <AiContext.Provider value={value}>{children}</AiContext.Provider>;
}

export function useAi(): AiState {
  const ctx = useContext(AiContext);
  if (!ctx) throw new Error('useAi must be used inside <AiProvider>');
  return ctx;
}

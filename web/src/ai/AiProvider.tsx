'use client';

// The AI layer (docs/frontend/README.md §3): conversation state, turn sending, the registry of
// elements people can ask about, and the screen commands the agent may run (highlight, scroll).
// Person-only actions are not here: they go through PersonButton and never through the agent.

import {createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode} from 'react';
import {useRouter} from 'next/navigation';

import {sendTurn, type TurnRequest} from './stream';
import type {AgentStep, ChatMessage, ElementContext, SavedMemory, Surface, TurnUsage, UiBlock} from './types';

interface AiState {
  surface: Surface;
  messages: ChatMessage[];
  blocks: UiBlock[];
  isBusy: boolean;
  /** Elements the person picked as "this" for the next message. */
  picked: ElementContext[];
  /** `taskId`: the task this message works on ("Ask the agent to work on it"). */
  send: (text: string, taskId?: string) => Promise<void>;
  /** A block's `say` button: sends its intent as `ui_event`, shown as `label` in the transcript. */
  sendEvent: (blockId: string, actionId: string, label: string, args?: Record<string, unknown>) => Promise<void>;
  /** Steps of the current (or last) turn. */
  steps: AgentStep[];
  /** Preferences remembered in this conversation, newest last. */
  memories: SavedMemory[];
  /** Latest state of each change set this conversation touched (`change_set.updated`). */
  changeSets: Record<string, {version: number; state: string}>;
  /** Usage of the last finished turn. */
  lastTurn: {usage: TurnUsage; models: string[]} | null;
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

  const router = useRouter();
  const [steps, setSteps] = useState<AgentStep[]>([]);
  const [memories, setMemories] = useState<SavedMemory[]>([]);
  const [changeSets, setChangeSets] = useState<Record<string, {version: number; state: string}>>({});
  const [lastTurn, setLastTurn] = useState<{usage: TurnUsage; models: string[]} | null>(null);

  const runCommand = useCallback(
    (command: {command?: string; target?: string}) => {
      // `open` goes to a page of this site; everything else only reaches registered targets,
      // and PersonButton never registers, so no command can press a person's button.
      if (command.command === 'open') {
        if (command.target?.startsWith('/')) router.push(command.target);
        return;
      }
      const el = command.target ? targets.get(command.target) : undefined;
      if (!el) return;
      if (command.command === 'scroll_to' || command.command === 'highlight' || command.command === 'focus_input') {
        el.scrollIntoView({behavior: 'smooth', block: 'center'});
      }
      if (command.command === 'highlight') {
        el.dataset.acomHighlight = 'true';
        window.setTimeout(() => delete el.dataset.acomHighlight, 4000);
      }
      if (command.command === 'focus_input') {
        const input = el.matches('input, textarea') ? el : el.querySelector<HTMLElement>('input, textarea');
        input?.focus();
      }
    },
    [router, targets],
  );

  /** Runs one turn and folds its events into the state. */
  const runTurn = useCallback(
    async (turn: Omit<TurnRequest, 'conversation_id'>) => {
      setIsBusy(true);
      setSteps([]);
      let reply = '';
      const replyId = nextId('msg');
      try {
        for await (const ev of sendTurn(surface, {conversation_id: conversationId.current, ...turn})) {
          const data = (ev.data ?? {}) as Record<string, unknown>;
          switch (ev.event) {
            case 'turn.started':
              if (typeof data.conversation_id === 'string') conversationId.current = data.conversation_id;
              break;
            case 'agent.step': {
              const step = data as unknown as AgentStep;
              setSteps(all => [...all.filter(x => x.step_id !== step.step_id), step]);
              break;
            }
            case 'message.delta':
              if (typeof data.text === 'string') {
                reply += data.text;
                const text = reply;
                setMessages(m => [...m.filter(x => x.id !== replyId), {id: replyId, sender: 'assistant', text}]);
              }
              break;
            case 'message.done':
              if (typeof data.text === 'string' && data.text !== reply) {
                const text = data.text;
                reply = text;
                setMessages(m => [...m.filter(x => x.id !== replyId), {id: replyId, sender: 'assistant', text}]);
              }
              break;
            case 'ui.block': {
              const block = ev.data as UiBlock;
              setBlocks(b => [...b.filter(x => x.block_id !== block.block_id), block]);
              break;
            }
            case 'memory.saved': {
              const memory = data as unknown as SavedMemory;
              setMemories(all => [...all.filter(x => x.memory_id !== memory.memory_id), memory]);
              break;
            }
            case 'change_set.updated':
              if (typeof data.change_set_id === 'string') {
                const id = data.change_set_id;
                setChangeSets(all => ({...all, [id]: {version: Number(data.version), state: String(data.state)}}));
              }
              break;
            case 'ui.command':
              runCommand(data as {command?: string; target?: string});
              break;
            case 'turn.done':
              setLastTurn({
                usage: (data.usage ?? {}) as TurnUsage,
                models: Array.isArray(data.models) ? data.models.map(String) : [],
              });
              break;
            case 'error': {
              const message = typeof data.message === 'string' ? data.message : 'Something went wrong.';
              setMessages(m => [...m, {id: nextId('msg'), sender: 'system', text: message}]);
              break;
            }
          }
        }
      } catch {
        setMessages(m => [...m, {id: nextId('msg'), sender: 'system', text: 'The assistant could not be reached.'}]);
      } finally {
        setIsBusy(false);
      }
    },
    [runCommand, surface],
  );

  const send = useCallback(
    async (text: string, taskId?: string) => {
      const context = picked;
      setMessages(m => [...m, {id: nextId('msg'), sender: 'user', text, context}]);
      setPicked([]);
      await runTurn({message: text, context, task_id: taskId});
    },
    [picked, runTurn],
  );

  const sendEvent = useCallback(
    async (blockId: string, actionId: string, label: string, args?: Record<string, unknown>) => {
      setMessages(m => [...m, {id: nextId('msg'), sender: 'user', text: label}]);
      await runTurn({ui_event: {block_id: blockId, action_id: actionId, args}});
    },
    [runTurn],
  );

  const pick = useCallback((element: ElementContext) => {
    setPicked(p => (p.some(x => x.element_ref === element.element_ref) ? p : [...p, element]));
  }, []);
  const unpick = useCallback((ref: string) => setPicked(p => p.filter(x => x.element_ref !== ref)), []);

  const value = useMemo(
    () => ({
      surface,
      messages,
      blocks,
      isBusy,
      picked,
      send,
      sendEvent,
      pick,
      unpick,
      targets,
      steps,
      memories,
      changeSets,
      lastTurn,
    }),
    [surface, messages, blocks, isBusy, picked, send, sendEvent, pick, unpick, targets, steps, memories, changeSets, lastTurn],
  );
  return <AiContext.Provider value={value}>{children}</AiContext.Provider>;
}

export function useAi(): AiState {
  const ctx = useContext(AiContext);
  if (!ctx) throw new Error('useAi must be used inside <AiProvider>');
  return ctx;
}

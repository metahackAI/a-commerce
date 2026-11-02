'use client';

// The AI layer (docs/frontend/README.md §3): conversation state, turn sending, the registry of
// elements people can ask about, and the screen commands the agent may run (highlight, scroll).
// Person-only actions are not here: they go through PersonButton and never through the agent.

import {createContext, useCallback, useContext, useMemo, useRef, useState, type ReactNode} from 'react';
import {useRouter} from 'next/navigation';

import {sendTurn, type TurnRequest} from './stream';
import type {AgentStep, ChatMessage, ElementContext, SavedMemory, Surface, TurnUsage, UiBlock} from './types';

/** What goes to the server for a picked element: `title` is the web's own and is left out. */
const wireContext = (context: ElementContext[]) =>
  context.map(c => ({element_ref: c.element_ref, kind: c.kind, label: c.label, ids: c.ids}));

interface AiState {
  surface: Surface;
  messages: ChatMessage[];
  /** Every block of the conversation, oldest first. */
  blocks: UiBlock[];
  /** Turns that sent blocks, oldest first. */
  resultTurns: string[];
  /** The blocks one turn sent. */
  blocksOf: (turnId: string) => UiBlock[];
  /** The turn whose blocks the canvas shows: the one a person chose, else the latest with blocks. */
  canvasTurn: string | null;
  /** Shows a turn's blocks on the canvas; `null` follows the latest turn again. */
  showOnCanvas: (turnId: string | null) => void;
  isBusy: boolean;
  /** The turn that is running, while one is. */
  liveTurn: string | null;
  /** Elements the person picked as "this" for the next message. */
  picked: ElementContext[];
  /**
   * `taskId`: the task this message works on ("Ask the agent to work on it"). `about`: elements
   * the message is about besides the picked ones, such as the element a suggested sentence belongs
   * to ("Tell me more about this one" under one product).
   */
  send: (text: string, taskId?: string, about?: ElementContext[]) => Promise<void>;
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
  // The turn each block came in (block_id → turn id), and the turn a person put on the canvas.
  const [blockTurns, setBlockTurns] = useState<Record<string, string>>({});
  const [chosenTurn, setChosenTurn] = useState<string | null>(null);
  const [isBusy, setIsBusy] = useState(false);
  const [liveTurn, setLiveTurn] = useState<string | null>(null);
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
    async (turnId: string, turn: Omit<TurnRequest, 'conversation_id'>) => {
      setIsBusy(true);
      setLiveTurn(turnId);
      setSteps([]);
      let reply = '';
      let sentBlocks = false;
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
                setMessages(m => [...m.filter(x => x.id !== replyId), {id: replyId, sender: 'assistant', text, turn_id: turnId}]);
              }
              break;
            case 'message.done':
              if (typeof data.text === 'string' && data.text !== reply) {
                const text = data.text;
                reply = text;
                setMessages(m => [...m.filter(x => x.id !== replyId), {id: replyId, sender: 'assistant', text, turn_id: turnId}]);
              }
              break;
            case 'ui.block': {
              const block = ev.data as UiBlock;
              setBlocks(b => [...b.filter(x => x.block_id !== block.block_id), block]);
              setBlockTurns(t => ({...t, [block.block_id]: turnId}));
              // A turn's first block puts the turn on the canvas, over whatever a person chose.
              if (!sentBlocks) {
                sentBlocks = true;
                setChosenTurn(null);
              }
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
              setMessages(m => [...m, {id: nextId('msg'), sender: 'system', text: message, turn_id: turnId}]);
              break;
            }
          }
        }
      } catch {
        setMessages(m => [...m, {id: nextId('msg'), sender: 'system', text: 'The assistant could not be reached.', turn_id: turnId}]);
      } finally {
        setIsBusy(false);
        setLiveTurn(null);
      }
    },
    [runCommand, surface],
  );

  const send = useCallback(
    async (text: string, taskId?: string, about: ElementContext[] = []) => {
      const context = [...picked, ...about.filter(a => !picked.some(p => p.element_ref === a.element_ref))];
      const turnId = nextId('turn');
      setMessages(m => [...m, {id: nextId('msg'), sender: 'user', text, context, turn_id: turnId}]);
      setPicked([]);
      await runTurn(turnId, {message: text, context: wireContext(context), task_id: taskId});
    },
    [picked, runTurn],
  );

  const sendEvent = useCallback(
    async (blockId: string, actionId: string, label: string, args?: Record<string, unknown>) => {
      const turnId = nextId('turn');
      setMessages(m => [...m, {id: nextId('msg'), sender: 'user', text: label, turn_id: turnId}]);
      await runTurn(turnId, {ui_event: {block_id: blockId, action_id: actionId, args}});
    },
    [runTurn],
  );

  const pick = useCallback((element: ElementContext) => {
    setPicked(p => (p.some(x => x.element_ref === element.element_ref) ? p : [...p, element]));
  }, []);
  const unpick = useCallback((ref: string) => setPicked(p => p.filter(x => x.element_ref !== ref)), []);

  // Blocks arrive in turn order (a block sent again moves to the turn that sent it), so the turns
  // in the order their blocks sit are the turns in the order they ran.
  const resultTurns = useMemo(() => {
    const turns: string[] = [];
    for (const b of blocks) {
      const t = blockTurns[b.block_id];
      if (t && !turns.includes(t)) turns.push(t);
    }
    return turns;
  }, [blocks, blockTurns]);
  const blocksOf = useCallback((turnId: string) => blocks.filter(b => blockTurns[b.block_id] === turnId), [blocks, blockTurns]);
  const canvasTurn = chosenTurn && resultTurns.includes(chosenTurn) ? chosenTurn : (resultTurns[resultTurns.length - 1] ?? null);

  const value = useMemo(
    () => ({
      surface,
      messages,
      blocks,
      resultTurns,
      blocksOf,
      canvasTurn,
      showOnCanvas: setChosenTurn,
      isBusy,
      liveTurn,
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
    [surface, messages, blocks, resultTurns, blocksOf, canvasTurn, isBusy, liveTurn, picked, send, sendEvent, pick, unpick, targets, steps, memories, changeSets, lastTurn],
  );
  return <AiContext.Provider value={value}>{children}</AiContext.Provider>;
}

export function useAi(): AiState {
  const ctx = useContext(AiContext);
  if (!ctx) throw new Error('useAi must be used inside <AiProvider>');
  return ctx;
}

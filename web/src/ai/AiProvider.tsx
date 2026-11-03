'use client';

// The AI layer (docs/frontend/README.md §3): conversation state, turn sending, the registry of
// elements people can ask about, and the screen commands the agent may run (highlight, scroll).
// Person-only actions are not here: they go through PersonButton and never through the agent.

import {createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode} from 'react';
import {useRouter} from 'next/navigation';

import {loadConversation} from './history';
import {chatSocket, chatTurn, type Transport} from './socket';
import type {TurnRequest} from './stream';
import type {AgentStep, ChatMessage, ElementContext, SavedMemory, Surface, TurnUsage, UiBlock} from './types';

/** What goes to the server for a picked element: `title` is the web's own and is left out. */
const wireContext = (context: ElementContext[]) =>
  context.map(c => ({element_ref: c.element_ref, kind: c.kind, label: c.label, ids: c.ids}));

interface AiState {
  surface: Surface;
  /** The conversation on screen, once its first turn has started (or it was opened). */
  conversationId: string | null;
  /** Changes when a turn ends or a conversation opens or starts, so history lists read again. */
  historyStamp: number;
  /** Shows an earlier conversation (its messages and blocks) to carry on from; one of the caller's own. */
  openConversation: (conversationId: string) => Promise<void>;
  /** Clears the screen for a new conversation; the last one stays in the history. */
  newConversation: () => void;
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
  /** Stops the turn that is running (the agent stops too). */
  stop: () => void;
  /** What carries the conversation: the WebSocket, or the HTTP event stream as a fallback. */
  transport: Transport;
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
  // The same, readable inside a running turn without waiting for a render.
  const blockTurnsRef = useRef<Record<string, string>>({});
  const [chosenTurn, setChosenTurn] = useState<string | null>(null);
  const [isBusy, setIsBusy] = useState(false);
  const [transport, setTransport] = useState<Transport>('idle');
  const running = useRef<AbortController | null>(null);
  useEffect(() => chatSocket(surface).onTransport(setTransport), [surface]);
  const [liveTurn, setLiveTurn] = useState<string | null>(null);
  const [picked, setPicked] = useState<ElementContext[]>([]);
  const conversationId = useRef<string | undefined>(undefined);
  const [conversation, setConversation] = useState<string | null>(null);
  const [historyStamp, setHistoryStamp] = useState(0);
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
      const abort = new AbortController();
      running.current = abort;
      // The reply as it stands on screen: written as it streams, taken off when a check resets it.
      const showReply = (text: string) =>
        setMessages(m => {
          const at = m.findIndex(x => x.id === replyId);
          if (!text) return at < 0 ? m : m.filter(x => x.id !== replyId);
          const msg: ChatMessage = {id: replyId, sender: 'assistant', text, turn_id: turnId};
          return at < 0 ? [...m, msg] : m.map(x => (x.id === replyId ? msg : x));
        });
      try {
        for await (const ev of chatTurn(surface, {conversation_id: conversationId.current, ...turn}, abort.signal)) {
          const data = (ev.data ?? {}) as Record<string, unknown>;
          switch (ev.event) {
            case 'turn.started':
              if (typeof data.conversation_id === 'string') {
                conversationId.current = data.conversation_id;
                setConversation(data.conversation_id);
              }
              break;
            case 'agent.step': {
              const step = data as unknown as AgentStep;
              setSteps(all => [...all.filter(x => x.step_id !== step.step_id), step]);
              break;
            }
            case 'message.delta':
              // The reply as the model writes it.
              if (typeof data.text === 'string') {
                reply += data.text;
                showReply(reply);
              }
              break;
            case 'message.reset':
              // A check turned the reply down; another streams next, or message.done brings it.
              reply = '';
              showReply('');
              break;
            case 'message.done':
              // The reply that stands, which a check may have changed from what streamed.
              if (typeof data.text === 'string' && data.text !== reply) {
                reply = data.text;
                showReply(reply);
              }
              break;
            case 'ui.block': {
              // While a turn works it sends a block again with the same id as it fills in: it
              // replaces itself where it is. A block an earlier turn sent moves to this turn.
              const block = ev.data as UiBlock;
              const mine = blockTurnsRef.current[block.block_id] === turnId;
              setBlocks(b =>
                mine && b.some(x => x.block_id === block.block_id)
                  ? b.map(x => (x.block_id === block.block_id ? block : x))
                  : [...b.filter(x => x.block_id !== block.block_id), block],
              );
              blockTurnsRef.current = {...blockTurnsRef.current, [block.block_id]: turnId};
              setBlockTurns(t => ({...t, [block.block_id]: turnId}));
              // A turn's first block puts the turn on the canvas, over whatever a person chose.
              if (!sentBlocks) {
                sentBlocks = true;
                setChosenTurn(null);
              }
              break;
            }
            case 'ui.block.removed':
              // A block shown while the turn worked that its answer does not keep.
              if (typeof data.block_id === 'string') {
                const id = data.block_id;
                setBlocks(b => b.filter(x => x.block_id !== id));
              }
              break;
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
              // A shopper's turn writes without approval (a cart line, a saved size): the
              // storefront's pages read the cart and orders on the server, so they read again.
              if (surface === 'fo') router.refresh();
              break;
            case 'error': {
              const message = typeof data.message === 'string' ? data.message : 'Something went wrong.';
              setMessages(m => [...m, {id: nextId('msg'), sender: 'system', text: message, turn_id: turnId}]);
              break;
            }
          }
        }
      } catch {
        if (!abort.signal.aborted) {
          setMessages(m => [...m, {id: nextId('msg'), sender: 'system', text: 'The assistant could not be reached.', turn_id: turnId}]);
        }
      } finally {
        if (abort.signal.aborted) setMessages(m => [...m, {id: nextId('msg'), sender: 'system', text: 'Stopped.', turn_id: turnId}]);
        running.current = null;
        setIsBusy(false);
        setLiveTurn(null);
        setHistoryStamp(n => n + 1);
      }
    },
    [runCommand, surface],
  );

  /** Puts a conversation on screen: `turns` as messages and blocks, or nothing for a new one. */
  const show = useCallback((id: string | null, turns: {turn_id: string; text: string; reply_text: string | null; blocks: UiBlock[] | null}[]) => {
    const shown: ChatMessage[] = [];
    let shownBlocks: UiBlock[] = [];
    const turnOf: Record<string, string> = {};
    for (const t of turns) {
      shown.push({id: nextId('msg'), sender: 'user', text: t.text, turn_id: t.turn_id});
      if (t.reply_text) shown.push({id: nextId('msg'), sender: 'assistant', text: t.reply_text, turn_id: t.turn_id});
      // A block sent again by a later turn belongs to that turn, as while the conversation ran.
      for (const b of t.blocks ?? []) {
        shownBlocks = [...shownBlocks.filter(x => x.block_id !== b.block_id), b];
        turnOf[b.block_id] = t.turn_id;
      }
    }
    conversationId.current = id ?? undefined;
    blockTurnsRef.current = turnOf;
    setConversation(id);
    setMessages(shown);
    setBlocks(shownBlocks);
    setBlockTurns(turnOf);
    setChosenTurn(null);
    setPicked([]);
    setSteps([]);
    setMemories([]);
    setChangeSets({});
    setLastTurn(null);
    setHistoryStamp(n => n + 1);
    // The address names an opened conversation, so a reload shows it again; a new one has none.
    const url = new URL(window.location.href);
    if (id) url.searchParams.set('conversation', id);
    else url.searchParams.delete('conversation');
    window.history.replaceState(window.history.state, '', url);
  }, []);

  const openConversation = useCallback(
    async (id: string) => {
      if (running.current || id === conversationId.current) return;
      try {
        const {turns} = await loadConversation(surface, id);
        show(id, turns);
      } catch {
        setMessages(m => [
          ...m,
          {id: nextId('msg'), sender: 'system', text: 'That conversation could not be opened. It may belong to someone else, or the demo store was loaded again.'},
        ]);
      }
    },
    [show, surface],
  );

  const newConversation = useCallback(() => {
    if (!running.current) show(null, []);
  }, [show]);

  const stop = useCallback(() => running.current?.abort(), []);

  const send = useCallback(
    async (text: string, taskId?: string, about: ElementContext[] = []) => {
      // One turn at a time: a message sent while one runs waits for the person to stop it.
      if (running.current) return;
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
      conversationId: conversation,
      historyStamp,
      openConversation,
      newConversation,
      messages,
      blocks,
      resultTurns,
      blocksOf,
      canvasTurn,
      showOnCanvas: setChosenTurn,
      isBusy,
      stop,
      transport,
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
    [surface, conversation, historyStamp, openConversation, newConversation, messages, blocks, resultTurns, blocksOf, canvasTurn, isBusy, stop, transport, liveTurn, picked, send, sendEvent, pick, unpick, targets, steps, memories, changeSets, lastTurn],
  );
  return <AiContext.Provider value={value}>{children}</AiContext.Provider>;
}

export function useAi(): AiState {
  const ctx = useContext(AiContext);
  if (!ctx) throw new Error('useAi must be used inside <AiProvider>');
  return ctx;
}

// The chat contract (docs/api/chat.md). These hand-written types are a placeholder: from M4 they
// are generated from the Rust contract into src/contract/ and imported from there instead.

export type Surface = 'bo' | 'fo';

/** An element a person picked to ask about (docs/api/chat.md §1, docs/frontend/README.md §3). */
export interface ElementContext {
  element_ref: string;
  kind: string;
  label: string;
  ids: Record<string, string>;
}

export type BlockAction =
  | {kind: 'say'; action_id: string; label: string; args?: Record<string, unknown>}
  | {kind: 'person'; action_id: string; label: string; method: string; path: string; body?: unknown; confirm: string}
  | {kind: 'open'; action_id: string; label: string; target: string};

export interface UiBlock {
  block_id: string;
  ui: string;
  schema: number;
  title?: string;
  why?: string;
  data: unknown;
  actions: BlockAction[];
  suggested_intents?: {element: string; say: string}[];
}

export interface ChatMessage {
  id: string;
  sender: 'user' | 'assistant' | 'system';
  text: string;
  context?: ElementContext[];
}

/** One server-sent event from `POST /api/chat/{surface}/turns`. */
export interface StreamEvent {
  id?: string;
  event: string;
  data: unknown;
}

/** Progress of one step of a turn (`agent.step`). */
export interface AgentStep {
  step_id: string;
  kind: 'route' | 'search_tools' | 'plan' | 'call_tool' | 'check' | string;
  summary: string;
  tool?: string;
  status: 'running' | 'done' | 'failed' | string;
}

/** A preference the agent remembered (`memory.saved`); a person can undo it. */
export interface SavedMemory {
  memory_id: string;
  text: string;
}

/** What a finished turn cost (`turn.done`). */
export interface TurnUsage {
  model_calls?: number;
  tool_calls?: number;
  input_tokens?: number;
  output_tokens?: number;
  ms?: number;
  cost_usd?: number;
}

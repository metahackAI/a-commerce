// Shared by the Ailism Bot pages on the server and their client views (a constant exported from a
// 'use client' module reaches a server page only as a client reference, not as the value).

/** The views of AI decisions, each the decision states it lists (`bot_decision_state`). */
export const DECISION_VIEWS: {value: string; label: string; states: string[]}[] = [
  {value: 'open', label: 'Needs a decision', states: ['waiting', 'needs_you', 'drafting']},
  {value: 'done', label: 'Live', states: ['live', 'running']},
  {value: 'turned_down', label: 'Turned down', states: ['rejected', 'dismissed', 'undone', 'failed']},
  {value: 'all', label: 'All', states: []},
];

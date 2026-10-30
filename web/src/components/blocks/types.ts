// One UI block from the agent (docs/api/chat.md §3) rendered as one component built from Astryx
// parts. A block component gets the block (its `data`, `actions`, `suggested_intents`) and must
// register its meaningful elements with `useAiTarget`.

import type {ComponentType} from 'react';

import type {UiBlock} from '@/ai/types';

export interface BlockProps {
  block: UiBlock;
}

export type BlockComponent = ComponentType<BlockProps>;

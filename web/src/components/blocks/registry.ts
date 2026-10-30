// Every block component by its `ui` name (docs/api/chat.md §3.3). Unknown blocks fall back to a
// card with the block's title and why-line.
import {boBlocks} from './bo';
import {foBlocks} from './fo';
import type {BlockComponent} from './types';

const blocks: Record<string, BlockComponent> = {...foBlocks, ...boBlocks};

export function blockComponent(ui: string): BlockComponent | undefined {
  return blocks[ui];
}

// Back-office blocks (docs/api/chat.md §3.3), by `ui` name.
import type {BlockComponent} from '../types';

import {ActionLogBlock} from './ActionLogBlock';
import {ChangeSetBlock} from './ChangeSetBlock';
import {ProductDraftsBlock} from './ProductDraftsBlock';
import {ProductTableBlock} from './ProductTableBlock';
import {RefundReviewBlock} from './RefundReviewBlock';
import {SalesExplainerBlock} from './SalesExplainerBlock';
import {TaskListBlock} from './TaskListBlock';

export const boBlocks: Record<string, BlockComponent> = {
  change_set: ChangeSetBlock,
  task_list: TaskListBlock,
  refund_review: RefundReviewBlock,
  product_drafts: ProductDraftsBlock,
  sales_explainer: SalesExplainerBlock,
  product_table: ProductTableBlock,
  action_log: ActionLogBlock,
};

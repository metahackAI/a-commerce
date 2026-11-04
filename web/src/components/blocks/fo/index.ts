// Storefront blocks (docs/api/chat.md §3.3), by `ui` name.
import type {BlockComponent} from '../types';

import {CartBlock} from './CartBlock';
import {CheckoutCardBlock} from './CheckoutCard';
import {CompareTableBlock} from './CompareTable';
import {FitCheckBlock} from './FitCheck';
import {GiftPickerBlock} from './GiftPicker';
import {LookGalleryBlock} from './LookGallery';
import {LookingForBlock} from './LookingFor';
import {MemoryNoteBlock} from './MemoryNote';
import {OrderDetailBlock} from './OrderDetail';
import {ProductDetailBlock} from './ProductDetail';
import {ProductListBlock} from './ProductList';
import {ProfileCardBlock} from './ProfileCard';
import {RecordListBlock} from './RecordList';
import {ReturnOptionsBlock} from './ReturnOptions';

export const foBlocks: Record<string, BlockComponent> = {
  looking_for: LookingForBlock,
  compare_table: CompareTableBlock,
  look_gallery: LookGalleryBlock,
  product_list: ProductListBlock,
  product_detail: ProductDetailBlock,
  record_list: RecordListBlock,
  fit_check: FitCheckBlock,
  gift_picker: GiftPickerBlock,
  cart: CartBlock,
  checkout_card: CheckoutCardBlock,
  return_options: ReturnOptionsBlock,
  order_detail: OrderDetailBlock,
  memory_note: MemoryNoteBlock,
  profile_card: ProfileCardBlock,
};

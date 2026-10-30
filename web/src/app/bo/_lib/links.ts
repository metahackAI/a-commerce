// Where things live in the back office: an entity id or a suggested next tool → a page.

/** The page for an entity id, by its prefix (docs/api/conventions.md §9), or `null`. */
export function entityHref(id: string | null | undefined): string | null {
  if (!id) return null;
  const prefix = id.slice(0, id.indexOf('_'));
  const enc = encodeURIComponent(id);
  switch (prefix) {
    case 'cs':
      return `/bo/change-sets/${enc}`;
    case 'task':
      return `/bo/tasks/${enc}`;
    case 'prd':
      return `/bo/products/${enc}`;
    case 'promo':
      return `/bo/promotions/${enc}`;
    case 'cpn':
      return `/bo/promotions?view=coupons&coupon=${enc}`;
    case 'ord':
      return `/bo/orders/${enc}`;
    case 'clm':
      return `/bo/claims/${enc}`;
    case 'cus':
      return `/bo/customers/${enc}`;
    case 'act':
      return `/bo/activity/${enc}`;
    case 'slot':
      return `/bo/merchandising?slot=${enc}`;
    default:
      return null;
  }
}

/** The page that answers an error's `next_tool`, with a short label, or `null`. */
export function nextToolLink(tool: string | null | undefined): {href: string; label: string} | null {
  switch (tool) {
    case 'search_change_sets':
    case 'get_change_set':
      return {href: '/bo/change-sets', label: 'Open change sets'};
    case 'list_tasks':
    case 'get_task':
      return {href: '/bo', label: 'Open the inbox'};
    case 'search_products':
    case 'search_product_groups':
    case 'get_product':
      return {href: '/bo/products', label: 'Find the product'};
    case 'search_promotions':
    case 'get_promotion':
    case 'list_active_promotions':
      return {href: '/bo/promotions', label: 'Open promotions'};
    case 'search_coupons':
      return {href: '/bo/promotions?view=coupons', label: 'Open coupons'};
    case 'list_display_slots':
    case 'list_banners':
    case 'list_categories':
      return {href: '/bo/merchandising', label: 'Open merchandising'};
    case 'search_orders':
    case 'get_order':
    case 'get_my_orders':
      return {href: '/bo/orders', label: 'Find the order'};
    case 'search_return_requests':
    case 'search_claims':
    case 'get_claim':
      return {href: '/bo/returns', label: 'Open returns'};
    case 'search_members':
      return {href: '/bo/customers', label: 'Find the customer'};
    case 'list_tiers':
      return {href: '/bo/customers', label: 'See the tiers'};
    case 'list_policies':
    case 'get_policy':
      return {href: '/bo/policies', label: 'Open policies'};
    case 'search_action_log':
      return {href: '/bo/activity', label: 'Open activity'};
    case 'describe_data':
      return {href: '/bo/data', label: 'Open the data dictionary'};
    case 'list_enums':
      return {href: '/bo/data', label: 'See the allowed values'};
    default:
      return null;
  }
}

/** A URL with search params replaced; empty values are dropped. */
export function withParams(base: string, params: Record<string, string | string[] | undefined | null>): string {
  const sp = new URLSearchParams();
  for (const [k, v] of Object.entries(params)) {
    if (v === undefined || v === null || v === '') continue;
    if (Array.isArray(v)) v.filter(Boolean).forEach(x => sp.append(k, x));
    else sp.append(k, v);
  }
  const s = sp.toString();
  return s ? `${base}?${s}` : base;
}

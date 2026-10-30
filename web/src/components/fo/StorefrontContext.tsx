'use client';

// What every storefront element may need to know: who is shopping (from the gateway session), how
// many lines the cart holds (from get_cart), the assistant drawer, and the products picked for
// comparison. The layout reads the first two on the server and hands them down.

import {createContext, useCallback, useContext, useEffect, useMemo, useState, type ReactNode} from 'react';

import type {Session} from '@/lib/session';

export interface ShopperInfo {
  session: Session;
  /** A signed-in shopper (orders, wishlist, memories). */
  isShopper: boolean;
  /** A guest session: may browse and keep a cart. */
  isGuest: boolean;
  name: string | null;
  tierId: string | null;
}

export interface CompareEntry {
  productId: string;
  name: string;
}

interface StorefrontState extends ShopperInfo {
  cartLines: number;
  isAssistantOpen: boolean;
  openAssistant: () => void;
  setAssistantOpen: (open: boolean) => void;
  compare: CompareEntry[];
  toggleCompare: (entry: CompareEntry) => void;
  clearCompare: () => void;
}

const Ctx = createContext<StorefrontState | null>(null);

const COMPARE_KEY = 'acom-compare';
/** compare_products takes 2–6 products (search.md). */
export const COMPARE_MAX = 6;

function readCompare(): CompareEntry[] {
  try {
    const raw = window.sessionStorage.getItem(COMPARE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : [];
    return Array.isArray(parsed) ? (parsed as CompareEntry[]).slice(0, COMPARE_MAX) : [];
  } catch {
    return [];
  }
}

export function StorefrontProvider({
  shopper,
  cartLines,
  children,
}: {
  shopper: ShopperInfo;
  cartLines: number;
  children: ReactNode;
}) {
  const [isAssistantOpen, setAssistantOpen] = useState(false);
  const [compare, setCompare] = useState<CompareEntry[]>([]);

  useEffect(() => setCompare(readCompare()), []);

  const save = useCallback((next: CompareEntry[]) => {
    setCompare(next);
    try {
      window.sessionStorage.setItem(COMPARE_KEY, JSON.stringify(next));
    } catch {
      // Storage may be unavailable (private mode); the list still works for this page.
    }
  }, []);

  const toggleCompare = useCallback(
    (entry: CompareEntry) => {
      const has = compare.some(c => c.productId === entry.productId);
      if (has) save(compare.filter(c => c.productId !== entry.productId));
      else if (compare.length < COMPARE_MAX) save([...compare, entry]);
    },
    [compare, save],
  );

  const value = useMemo<StorefrontState>(
    () => ({
      ...shopper,
      cartLines,
      isAssistantOpen,
      openAssistant: () => setAssistantOpen(true),
      setAssistantOpen,
      compare,
      toggleCompare,
      clearCompare: () => save([]),
    }),
    [shopper, cartLines, isAssistantOpen, compare, toggleCompare, save],
  );
  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useStorefront(): StorefrontState {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error('useStorefront must be used inside <StorefrontProvider>');
  return ctx;
}

'use client';

// A block the agent is putting on the canvas comes together piece by piece instead of all at once:
// its card first, then its title, rows, pictures and buttons in reading order, quickly. It works on
// the block's elements rather than knowing each block: the AI targets mark what the pieces are, so
// every block, and any block added later, assembles the same way. Only what arrives while a turn
// runs assembles; a turn a person flips back to just shows.

import {useEffect, useLayoutEffect, useRef, type ReactNode, type RefObject} from 'react';

/** Between one piece and the next, in ms; a large block shortens it so it never takes longer than MOST. */
const STEP = 40;
const MOST = 800;
/** One piece's own animation, in ms (`acom-part` in the workspace's CSS). */
const PIECE = 280;
/** Content a block reads for itself (a product's page) can come after the turn ends: watch this long, in ms. */
const LATE = 4000;
/** An element taller than this, with several children, is more than one piece. */
const TALL = 140;

/** Wrappers with a single child say nothing: the element that actually holds the pieces. */
function unwrap(el: Element): Element {
  let at = el;
  while (at.children.length === 1 && at.firstElementChild) at = at.firstElementChild;
  return at;
}

function visible(el: Element): el is HTMLElement {
  return el instanceof HTMLElement && el.tagName !== 'STYLE' && el.tagName !== 'SCRIPT' && el.getClientRects().length > 0;
}

/** The pieces under `el`, in reading order. */
function collect(el: Element, into: HTMLElement[]): void {
  for (const child of Array.from(el.children)) {
    if (!visible(child)) continue;
    const isTarget = child.hasAttribute('data-acom-ref');
    if (isTarget && child.getAttribute('data-acom-kind') !== 'block') {
      // One thing a person can point at (a product, a row): its parts come in one after another.
      const inner = Array.from(unwrap(child).children).filter(visible);
      if (inner.length > 1) into.push(...inner);
      else into.push(child);
    } else if (isTarget || child.querySelector('[data-acom-ref]') || (child.offsetHeight > TALL && child.children.length > 1) || /^(TABLE|THEAD|TBODY|TFOOT|UL|OL)$/.test(child.tagName)) {
      collect(child.children.length === 1 ? (unwrap(child).parentElement ?? child) : child, into);
    } else {
      into.push(child);
    }
  }
}

export function Assembling({isLive, nextStart, children}: {isLive: boolean; nextStart: RefObject<number>; children: ReactNode}) {
  const root = useRef<HTMLDivElement | null>(null);
  const startedLive = useRef(isLive);
  const observer = useRef<MutationObserver | null>(null);

  useLayoutEffect(() => {
    const el = root.current;
    if (!el || !startedLive.current || window.matchMedia('(prefers-reduced-motion: reduce)').matches) return;

    const assemble = () => {
      const pieces: HTMLElement[] = [];
      collect(el, pieces);
      // Whatever already came in stays; only what is new assembles, inside nothing that already did.
      const fresh = pieces.filter(p => !p.closest('[data-acom-assembled]'));
      if (fresh.length === 0) return;
      const now = performance.now();
      const start = Math.max(0, nextStart.current - now);
      const step = Math.min(STEP, MOST / fresh.length);
      fresh.forEach((p, i) => {
        p.setAttribute('data-acom-assembled', '');
        p.setAttribute('data-acom-part', '');
        p.style.setProperty('--acom-part-delay', `${Math.round(start + i * step)}ms`);
      });
      // Blocks sent together follow each other, each starting as the one before is finishing.
      nextStart.current = now + start + (fresh.length * step + PIECE) * 0.6;
    };
    // A finished piece goes back to being a plain element.
    const finished = (e: AnimationEvent) => {
      if (e.animationName !== 'acom-part' || !(e.target instanceof HTMLElement)) return;
      e.target.removeAttribute('data-acom-part');
      e.target.style.removeProperty('--acom-part-delay');
    };

    assemble();
    el.addEventListener('animationend', finished);
    // A block replaced while the turn runs, or one that reads its own content, gets new pieces.
    const watching = new MutationObserver(assemble);
    watching.observe(el, {childList: true, subtree: true});
    observer.current = watching;
    return () => {
      watching.disconnect();
      el.removeEventListener('animationend', finished);
    };
  }, [nextStart]);

  // After the turn, a person's own clicks in the block should not assemble anything.
  useEffect(() => {
    if (isLive) return;
    const t = window.setTimeout(() => observer.current?.disconnect(), LATE);
    return () => window.clearTimeout(t);
  }, [isLive]);

  return <div ref={root}>{children}</div>;
}

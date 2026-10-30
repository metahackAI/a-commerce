'use client';

// Registers an element as an AI target (docs/frontend/README.md §3.1): people can pick it as
// "this" in a question, and the agent can point at it with `ui.command`. The returned props go on
// the element; `data-acom-*` attributes also let browser agents and tests find it.

import {useCallback} from 'react';

import {useAi} from './AiProvider';
import type {ElementContext} from './types';

export interface AiTarget {
  kind: string;
  label: string;
  ids: Record<string, string>;
  blockId?: string;
  /** schema.org or acom term the element shows, e.g. `schema:Offer`. */
  term?: string;
}

export function elementRef(target: AiTarget): string {
  const main = Object.values(target.ids)[0] ?? target.label;
  return `${target.blockId ?? 'page'}#${target.kind}:${main}`;
}

export function useAiTarget(target: AiTarget) {
  const {targets, pick} = useAi();
  const ref = elementRef(target);

  const register = useCallback(
    (el: HTMLElement | null) => {
      if (el) targets.set(ref, el);
      else targets.delete(ref);
    },
    [ref, targets],
  );

  const context: ElementContext = {element_ref: ref, kind: target.kind, label: target.label, ids: target.ids};

  return {
    ref: register,
    /** Picks this element as "this" for the next message. */
    ask: () => pick(context),
    props: {
      'data-acom-kind': target.kind,
      'data-acom-id': Object.values(target.ids)[0],
      'data-acom-term': target.term,
      'data-acom-ref': ref,
    },
  };
}

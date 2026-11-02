'use client';

// The conversation history (docs/frontend/README.md §3): the caller's earlier conversations on
// this side of the store, where the conversation already is. Recent ones under the starters of an
// empty conversation, a history button and "new conversation" in a conversation's header, and the
// whole list on the storefront account page. Picking one shows it again to carry on from.

import {DropdownMenu, type DropdownMenuItemData} from '@astryxdesign/core/DropdownMenu';
import {Icon} from '@astryxdesign/core/Icon';
import {IconButton} from '@astryxdesign/core/IconButton';
import {HStack} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {ClockIcon, PencilSquareIcon} from '@heroicons/react/24/outline';
import {useEffect, useState} from 'react';

import {useAi} from '@/ai/AiProvider';
import {conversationName, listConversations, whenText, type ConversationSummary} from '@/ai/history';

// Quiet rows, as the starters look: borders and type only, the accent on focus.
const HISTORY_CSS = `
.acom-history { display: flex; flex-direction: column; gap: var(--spacing-1); width: 100%; max-width: 520px; }
.acom-history-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; }
.acom-history-row { all: unset; box-sizing: border-box; width: 100%; cursor: pointer; display: flex; align-items: baseline; gap: var(--spacing-3); padding: var(--spacing-2) var(--spacing-2); border-radius: var(--radius-container); }
.acom-history-row:hover { background: var(--color-background-muted); }
.acom-history-row:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 1px; }
.acom-history-row[aria-current="true"] { background: var(--color-background-muted); }
.acom-history-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--color-text-primary); font-size: var(--font-size-sm); }
.acom-history-when { flex-shrink: 0; color: var(--color-text-secondary); font-size: var(--font-size-xs, 12px); font-variant-numeric: tabular-nums; }
.acom-history-head { display: flex; align-items: center; justify-content: space-between; gap: var(--spacing-2); padding: 0 var(--spacing-2); }
`;

/** The caller's conversations on this surface; read again when a turn ends or one opens. */
export function useConversations(limit: number, enabled = true): {items: ConversationSummary[]; total: number} | null {
  const ai = useAi();
  const [list, setList] = useState<{items: ConversationSummary[]; total: number} | null>(null);
  useEffect(() => {
    if (!enabled) return;
    let live = true;
    listConversations(ai.surface, limit)
      .then(l => live && setList(l))
      // No list (signed out, or the store did not answer) shows no history rather than an error.
      .catch(() => live && setList({items: [], total: 0}));
    return () => {
      live = false;
    };
  }, [ai.surface, ai.historyStamp, limit, enabled]);
  return list;
}

/** Rows that open a conversation; the one on screen is marked. */
function Rows({items, onOpen}: {items: ConversationSummary[]; onOpen: (id: string) => void}) {
  const ai = useAi();
  return (
    <ul className="acom-history-list">
      {items.map(c => (
        <li key={c.conversation_id}>
          <button
            type="button"
            className="acom-history-row"
            aria-current={c.conversation_id === ai.conversationId}
            title={conversationName(c)}
            onClick={() => onOpen(c.conversation_id)}>
            <span className="acom-history-name">{conversationName(c)}</span>
            <span className="acom-history-when">{whenText(c.last_turn_at ?? c.started_at)}</span>
          </button>
        </li>
      ))}
    </ul>
  );
}

/** Under the starters of an empty conversation: the last few, and the rest behind a menu. */
export function RecentConversations({count = 3}: {count?: number}) {
  const ai = useAi();
  const list = useConversations(count);
  if (!list || list.items.length === 0) return null;
  return (
    <div className="acom-history" aria-label="Recent conversations">
      <style>{HISTORY_CSS}</style>
      <div className="acom-history-head">
        <Text type="label" weight="semibold" color="secondary">
          Recent conversations
        </Text>
        {list.total > list.items.length ? <HistoryMenu label={`All ${list.total}`} /> : null}
      </div>
      <Rows items={list.items} onOpen={id => void ai.openConversation(id)} />
    </div>
  );
}

/**
 * The history as a menu: an icon in a conversation's header, or a small text button. It reads the
 * list when it opens.
 */
export function HistoryMenu({label}: {label?: string}) {
  const ai = useAi();
  const [open, setOpen] = useState(false);
  const list = useConversations(20, open);
  const rows: DropdownMenuItemData[] =
    list === null
      ? [{label: 'Loading…', isDisabled: true}]
      : list.items.length === 0
        ? [{label: 'No conversations yet', isDisabled: true}]
        : list.items.map(c => ({
            id: c.conversation_id,
            label: conversationName(c),
            description: [whenText(c.last_turn_at ?? c.started_at), c.conversation_id === ai.conversationId ? 'On screen' : null]
              .filter(Boolean)
              .join(' · '),
            isDisabled: ai.isBusy,
            onClick: () => void ai.openConversation(c.conversation_id),
          }));
  return (
    <DropdownMenu
      button={
        label
          ? {label, variant: 'ghost', size: 'sm'}
          : {label: 'Conversation history', variant: 'ghost', size: 'sm', icon: <Icon icon={ClockIcon} />, isIconOnly: true, tooltip: 'Conversation history'}
      }
      alignment="end"
      menuWidth={340}
      menuMaxHeight={420}
      isMenuOpen={open}
      onOpenChange={setOpen}
      items={[{type: 'section', title: 'Conversations', items: rows}]}
    />
  );
}

/** Clears the screen for a new conversation; there is nothing to clear in an empty one. */
export function NewConversationButton() {
  const ai = useAi();
  return (
    <IconButton
      label="New conversation"
      tooltip="New conversation"
      variant="ghost"
      size="sm"
      icon={<Icon icon={PencilSquareIcon} />}
      isDisabled={ai.isBusy || (ai.messages.length === 0 && ai.blocks.length === 0)}
      onClick={() => ai.newConversation()}
    />
  );
}

/** The history and "new conversation", side by side, for a conversation's header. */
export function ConversationActions() {
  return (
    <HStack gap={0.5} vAlign="center">
      <HistoryMenu />
      <NewConversationButton />
    </HStack>
  );
}

/** Every conversation, as rows that go to `page` with it open (the storefront account page). */
export function ConversationList({page}: {page: string}) {
  const list = useConversations(50);
  if (list === null) {
    return (
      <Text type="supporting" color="secondary">
        Loading…
      </Text>
    );
  }
  if (list.items.length === 0) {
    return (
      <Text type="supporting" color="secondary">
        No conversations yet. Ask Ailism anything from the main page.
      </Text>
    );
  }
  return (
    <div className="acom-history" style={{maxWidth: 'none'}}>
      <style>{HISTORY_CSS}</style>
      <Rows items={list.items} onOpen={id => window.location.assign(`${page}?conversation=${encodeURIComponent(id)}`)} />
      {list.total > list.items.length ? (
        <Text type="supporting" color="secondary">
          The latest {list.items.length} of {list.total}.
        </Text>
      ) : null}
    </div>
  );
}

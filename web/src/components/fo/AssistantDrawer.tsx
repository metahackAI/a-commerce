'use client';

// The assistant on every storefront page: the same conversation as the main page (one AiProvider
// in the layout), in a drawer. Elements picked with "Ask about this" ride along as chips; the
// agent's UI blocks show in the conversation, drawn by the same block components as the canvas.
// The main page and /ask are the conversation themselves, so the drawer stays closed there.

import {Button} from '@astryxdesign/core/Button';
import {Drawer} from '@astryxdesign/core/Drawer';
import {Icon} from '@astryxdesign/core/Icon';
import {IconButton} from '@astryxdesign/core/IconButton';
import {XMarkIcon} from '@heroicons/react/24/outline';
import {usePathname} from 'next/navigation';

import {AILISM} from '@/components/brand/Ailism';
import {ChatEmpty, ChatHeader, ChatPanel} from '@/components/chat/ChatParts';
import {ConversationActions} from '@/components/chat/History';

import {useStorefront} from './StorefrontContext';

const STARTERS = ['I need a light jacket for spring trips.', 'What goes with what is in my cart?', 'Where is my order?'];

/** Pages that are the conversation themselves. */
export function isConversationPage(pathname: string): boolean {
  return pathname === '/' || pathname === '/ask';
}

export function AssistantDrawer() {
  const {isAssistantOpen, setAssistantOpen} = useStorefront();
  const pathname = usePathname();
  const isOpen = isAssistantOpen && !isConversationPage(pathname);

  return (
    <Drawer isOpen={isOpen} onOpenChange={setAssistantOpen} label="Ask Ailism" width={480} padding={0} experimental={{isFullWidthOnMobile: true}}>
      {/* Drawn only while open: elsewhere the same conversation is the page itself. */}
      {isOpen ? (
        <ChatPanel
          header={
            <ChatHeader
              subtitle={`${AILISM.screen} Only you can place an order.`}
              actions={
                <>
                  <ConversationActions />
                  <Button label="Full view" size="sm" variant="ghost" href="/" onClick={() => setAssistantOpen(false)} />
                  <IconButton label="Close" variant="ghost" size="sm" icon={<Icon icon={XMarkIcon} />} onClick={() => setAssistantOpen(false)} />
                </>
              }
            />
          }
          placeholder="Ask about fit, fabric, delivery, returns…"
          emptyState={<ChatEmpty title={AILISM.tagline} description="Ask in your own words, or pick something on the page and ask about it." starters={STARTERS} />}
        />
      ) : null}
    </Drawer>
  );
}

import {HStack} from '@astryxdesign/core/Layout';

import {Workspace, WorkspaceLink} from '@/components/workspace/Workspace';

const STARTERS = [
  'I need a light jacket for spring trips.',
  'A gift for my dad, under $100',
  'Will M fit me in the wool overcoat?',
  'Where is my order?',
];

/**
 * The storefront as a conversation (scenes F1–F6): the shopper says what they want and the page
 * is assembled from the agent's UI blocks beside the conversation. Browsing the store the usual
 * way is one link away.
 */
export function StorefrontWorkspace({name}: {name: string | null}) {
  return (
    <Workspace
      greeting={name ? `What are you looking for today, ${name}?` : 'What are you looking for today?'}
      intro="Say it once. Ailism finds, compares and prepares the page for you; only you place the order."
      composerPlaceholder="Ask about fit, fabric, delivery, returns…"
      canvasTitle="For you"
      starters={STARTERS}
      belowStarters={
        <HStack gap={1} wrap="wrap" hAlign="center">
          <WorkspaceLink label="Browse the store" href="/shop" />
          <WorkspaceLink label="Shop all" href="/search" />
          <WorkspaceLink label="Collections" href="/collections" />
        </HStack>
      }
    />
  );
}

import type {Metadata} from 'next';

import {Workspace} from '@/components/workspace/Workspace';

export const metadata: Metadata = {title: 'Ask Northwind'};

// The full conversation + canvas workspace (F1, F2, F6). The same conversation shows in the
// assistant drawer on every other page.
export default function AskPage() {
  return (
    <Workspace
      greeting="What are you looking for today?"
      composerPlaceholder="Ask about fit, fabric, delivery, returns…"
      canvasTitle="For you"
      canvasEmpty={{
        title: 'Results show up here',
        description: 'Comparisons, photos, and your cart appear here as you talk.',
      }}
    />
  );
}

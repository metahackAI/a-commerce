import {Workspace} from '@/components/workspace/Workspace';

export default function StorefrontPage() {
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

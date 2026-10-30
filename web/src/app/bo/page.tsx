import {Workspace} from '@/components/workspace/Workspace';

export default function BackOfficeHome() {
  return (
    <Workspace
      greeting="What should the store do next?"
      composerPlaceholder="Give the agent a task…"
      canvasTitle="Change set"
      canvasEmpty={{
        title: 'Nothing to review yet',
        description: 'Change sets, return reviews, and explanations appear here. Nothing changes in the store until you approve.',
      }}
    />
  );
}

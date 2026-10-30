import {Center} from '@astryxdesign/core/Center';
import {Spinner} from '@astryxdesign/core/Spinner';

// Shown while a back-office page reads from the store.
export default function Loading() {
  return (
    <Center axis="both" padding={10} minHeight="60dvh">
      <Spinner size="lg" label="Loading…" />
    </Center>
  );
}

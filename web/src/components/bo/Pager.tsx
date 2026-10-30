// Cursor paging (docs/api/conventions.md §6): how many rows of the total are shown, and links to
// the first and the next page. The next page keeps every other search param of the URL.

import {HStack, StackItem} from '@astryxdesign/core/Layout';
import {Text} from '@astryxdesign/core/Text';
import {Button} from '@astryxdesign/core/Button';

import {count} from '@/app/bo/_lib/format';
import {withParams} from '@/app/bo/_lib/links';

export function Pager({
  base,
  params,
  shown,
  total,
  nextCursor,
  noun = 'rows',
}: {
  base: string;
  params: Record<string, string | string[] | undefined>;
  shown: number;
  total: number;
  nextCursor?: string | null;
  noun?: string;
}) {
  const onLaterPage = Boolean(params.cursor);
  return (
    <HStack gap={2} vAlign="center" wrap="wrap">
      <StackItem size="fill">
        <Text type="supporting" color="secondary">
          {shown === total && !onLaterPage ? `${count(total)} ${noun}` : `${count(shown)} shown of ${count(total)} ${noun}`}
        </Text>
      </StackItem>
      {onLaterPage ? <Button label="First page" size="sm" variant="ghost" href={withParams(base, {...params, cursor: undefined})} /> : null}
      {nextCursor ? <Button label="Next page" size="sm" href={withParams(base, {...params, cursor: nextCursor})} /> : null}
    </HStack>
  );
}

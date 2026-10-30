'use client';

// A product or banner picture in a fixed ratio. The API gives a URL (or nothing); without one, a
// muted placeholder keeps the grid even.

import {AspectRatio} from '@astryxdesign/core/AspectRatio';
import {Card} from '@astryxdesign/core/Card';
import {Center} from '@astryxdesign/core/Center';
import {Icon} from '@astryxdesign/core/Icon';
import {PhotoIcon} from '@heroicons/react/24/outline';

export function ProductImage({src, alt, ratio = 4 / 5}: {src: string | null | undefined; alt: string; ratio?: number}) {
  return (
    <Card padding={0} variant="muted">
      <AspectRatio ratio={ratio} fit="cover">
        {src ? (
          <img src={src} alt={alt} loading="lazy" />
        ) : (
          <Center>
            <Icon icon={PhotoIcon} color="disabled" size="lg" label={`${alt}: no photo yet`} />
          </Center>
        )}
      </AspectRatio>
    </Card>
  );
}

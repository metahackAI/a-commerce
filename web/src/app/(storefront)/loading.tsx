import {Grid} from '@astryxdesign/core/Grid';
import {Layout, LayoutContent, VStack} from '@astryxdesign/core/Layout';
import {Skeleton} from '@astryxdesign/core/Skeleton';

// Shown while a storefront page reads the store (session, prices, stock).
export default function Loading() {
  return (
    <Layout
      height="auto"
      contentWidth={1200}
      content={
        <LayoutContent padding={6}>
          <VStack gap={5}>
            <Skeleton height={40} width="40%" />
            <Grid columns={{minWidth: 220}} gap={5}>
              {[0, 1, 2, 3].map(i => (
                <VStack key={i} gap={2}>
                  <Skeleton height={260} index={i} />
                  <Skeleton height={20} width="70%" index={i} />
                  <Skeleton height={20} width="40%" index={i} />
                </VStack>
              ))}
            </Grid>
          </VStack>
        </LayoutContent>
      }
    />
  );
}

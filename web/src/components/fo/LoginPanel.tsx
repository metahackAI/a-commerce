'use client';

// Shopper sign-in (docs/backend/gateway.md §6). In demo mode the shoppers from
// /api/auth/personas sign in with one press; anyone can continue as a guest, who may browse and
// keep a cart but needs to sign in to order.

import {Banner} from '@astryxdesign/core/Banner';
import {Button} from '@astryxdesign/core/Button';
import {Card} from '@astryxdesign/core/Card';
import {Divider} from '@astryxdesign/core/Divider';
import {List, ListItem} from '@astryxdesign/core/List';
import {VStack} from '@astryxdesign/core/Layout';
import {Skeleton} from '@astryxdesign/core/Skeleton';
import {Heading, Text} from '@astryxdesign/core/Text';
import {useRouter} from 'next/navigation';
import {useEffect, useState} from 'react';

import type {ApiErrorBody} from '@/lib/api';
import {continueAsGuest, demoSignIn, listPersonas, signOut, type Persona} from '@/lib/session';

import {AilismAvatar} from '@/components/brand/Ailism';
import {errorBody} from './write';
import {useStorefront} from './StorefrontContext';

export function LoginPanel({next}: {next: string}) {
  const router = useRouter();
  const {isShopper, isGuest, name} = useStorefront();
  const [personas, setPersonas] = useState<Persona[] | null>(null);
  const [personasError, setPersonasError] = useState<ApiErrorBody | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<ApiErrorBody | null>(null);

  useEffect(() => {
    listPersonas()
      .then(r => setPersonas(r.items.filter(p => p.kind === 'shopper')))
      .catch(e => {
        setPersonas([]);
        setPersonasError(errorBody(e));
      });
  }, []);

  async function run(key: string, action: () => Promise<unknown>) {
    setBusy(key);
    setError(null);
    try {
      await action();
      router.push(next);
      router.refresh();
    } catch (e) {
      setError(errorBody(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <VStack gap={4} width="100%" maxWidth={440}>
      <VStack gap={2} hAlign="center">
        <AilismAvatar size="xl" />
        <Heading level={1}>Sign in to Ailism Commerce</Heading>
        <Text type="body" color="secondary" justify="center">
          Members see their own prices, saved sizes and orders. Guests can browse and keep a cart.
        </Text>
      </VStack>
      {isShopper || isGuest ? (
        <Banner
          status="info"
          title={isShopper ? `Signed in as ${name ?? 'a shopper'}` : 'You are browsing as a guest'}
          description={isShopper ? 'Pick someone else below, or sign out.' : 'Sign in to place an order; your cart stays with this session.'}
          endContent={
            isShopper ? (
              <Button
                label="Sign out"
                size="sm"
                clickAction={async () => {
                  await signOut().catch(() => undefined);
                  router.refresh();
                }}
              />
            ) : undefined
          }
        />
      ) : null}
      {error ? <Banner status="error" title={error.message} /> : null}
      <Card padding={6} width="100%">
        <VStack gap={4}>
          <VStack gap={2}>
            <Text type="label" weight="semibold">
              Demo shoppers
            </Text>
            {personas === null ? (
              <VStack gap={2}>
                <Skeleton height={48} />
                <Skeleton height={48} index={1} />
              </VStack>
            ) : personas.length === 0 ? (
              <Text type="supporting" color="secondary">
                {personasError
                  ? `${personasError.message} Shopper sign-in by persona works only in demo mode.`
                  : 'No demo shoppers are set up for this store.'}
              </Text>
            ) : (
              <List hasDividers density="spacious">
                {personas.map(p => (
                  <ListItem
                    key={p.persona}
                    label={p.name}
                    description={p.description ?? undefined}
                    endContent={
                      <Button
                        label={`Continue as ${p.name}`}
                        size="sm"
                        variant="primary"
                        isLoading={busy === p.persona}
                        isDisabled={busy !== null && busy !== p.persona}
                        onClick={() => void run(p.persona, () => demoSignIn(p.persona))}
                      />
                    }
                  />
                ))}
              </List>
            )}
          </VStack>
          <Divider label="or" />
          <VStack gap={2}>
            <Button
              label="Continue as guest"
              width="100%"
              isLoading={busy === 'guest'}
              isDisabled={busy !== null && busy !== 'guest'}
              onClick={() => void run('guest', continueAsGuest)}
            />
            <Text type="supporting" color="secondary" justify="center">
              No account needed to look around or fill a cart.
            </Text>
          </VStack>
        </VStack>
      </Card>
    </VStack>
  );
}

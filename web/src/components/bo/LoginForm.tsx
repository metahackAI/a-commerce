'use client';

// Operator sign-in: login id and password (POST /api/auth/login), and in demo mode the operator
// personas from /api/auth/personas as one-click sign-in. A session that isn't an operator's is
// not let into the back office.

import {useEffect, useState} from 'react';
import {useRouter} from 'next/navigation';
import {Center} from '@astryxdesign/core/Center';
import {VStack} from '@astryxdesign/core/Layout';
import {Card} from '@astryxdesign/core/Card';
import {Heading, Text} from '@astryxdesign/core/Text';
import {TextInput} from '@astryxdesign/core/TextInput';
import {Button} from '@astryxdesign/core/Button';
import {Banner} from '@astryxdesign/core/Banner';
import {Divider} from '@astryxdesign/core/Divider';
import {List, ListItem} from '@astryxdesign/core/List';
import {Spinner} from '@astryxdesign/core/Spinner';
import {Avatar} from '@astryxdesign/core/Avatar';

import {AILISM, AilismAvatar, AilismMark} from '@/components/brand/Ailism';
import {ApiError} from '@/lib/api';
import {demoSignIn, listPersonas, signIn, type Persona, type Session} from '@/lib/session';

function message(e: unknown): string {
  if (e instanceof ApiError) {
    if (e.code === 'invalid_credentials') return 'That login id and password don’t match an operator account.';
    return e.message;
  }
  return 'The store could not be reached. Try again in a moment.';
}

export function LoginForm({demoMode, signedInAs}: {demoMode: boolean; signedInAs: string | null}) {
  const router = useRouter();
  const [loginId, setLoginId] = useState('');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [personas, setPersonas] = useState<Persona[] | null>(null);
  const [personaError, setPersonaError] = useState<string | null>(null);

  useEffect(() => {
    if (!demoMode) return;
    let alive = true;
    listPersonas()
      .then(r => alive && setPersonas(r.items.filter(p => p.kind === 'operator')))
      .catch(e => alive && setPersonaError(message(e)));
    return () => {
      alive = false;
    };
  }, [demoMode]);

  function enter(session: Session) {
    if (session.user?.kind !== 'operator') {
      setError('This account is not an operator account. Sign in with an operator login.');
      return;
    }
    router.replace('/bo');
    router.refresh();
  }

  async function submit() {
    setError(null);
    if (!loginId || !password) {
      setError('Enter your login id and password.');
      return;
    }
    setBusy('password');
    try {
      enter(await signIn(loginId, password));
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(null);
    }
  }

  async function asPersona(p: Persona) {
    setError(null);
    setBusy(p.persona);
    try {
      enter(await demoSignIn(p.persona));
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(null);
    }
  }

  return (
    <Center axis="both" padding={6} minHeight="100dvh">
      <VStack gap={4} hAlign="center" width="100%" maxWidth={420}>
        <VStack gap={2} hAlign="center">
          <AilismAvatar size="xl" />
          <AilismMark size="lg" />
          <Text type="supporting" color="secondary">
            {`Back office · ${AILISM.promise}`}
          </Text>
        </VStack>

        <Card padding={8} width="100%">
          <VStack gap={4} hAlign="stretch">
            <VStack gap={1} hAlign="center">
              <Heading level={2}>Sign in</Heading>
              <Text type="body" color="secondary" size="sm">
                Operators only. Shoppers use the store.
              </Text>
            </VStack>

            {signedInAs ? (
              <Banner status="info" title={`You are signed in as ${signedInAs}, who is not an operator.`} description="Sign in with an operator account to continue." />
            ) : null}
            {error ? <Banner status="error" title={error} /> : null}

            <TextInput label="Login id" value={loginId} onChange={setLoginId} autoComplete="username" size="lg" hasAutoFocus onEnter={() => void submit()} />
            <TextInput
              label="Password"
              type="password"
              value={password}
              onChange={setPassword}
              autoComplete="current-password"
              size="lg"
              onEnter={() => void submit()}
            />
            <Button label="Sign in" variant="primary" size="lg" isLoading={busy === 'password'} onClick={() => void submit()} />

            {demoMode ? (
              <VStack gap={3}>
                <Divider />
                <VStack gap={1}>
                  <Text type="label" weight="semibold">
                    Demo operators
                  </Text>
                  <Text type="supporting" color="secondary">
                    This store runs in demo mode. Sign in as one of its operators with one click.
                  </Text>
                </VStack>
                {personaError ? <Banner status="warning" title={personaError} /> : null}
                {personas === null && !personaError ? <Spinner size="sm" label="Loading operators" /> : null}
                {personas && personas.length === 0 ? (
                  <Text type="supporting" color="secondary">
                    No demo operators are set up.
                  </Text>
                ) : null}
                {personas && personas.length > 0 ? (
                  <List hasDividers density="compact">
                    {personas.map(p => (
                      <ListItem
                        key={p.persona}
                        label={p.name}
                        description={p.description ?? p.login_id ?? p.persona}
                        startContent={<Avatar name={p.name} size="sm" />}
                        endContent={
                          <Button
                            label="Sign in"
                            size="sm"
                            isLoading={busy === p.persona}
                            onClick={() => void asPersona(p)}
                          />
                        }
                      />
                    ))}
                  </List>
                ) : null}
              </VStack>
            ) : null}
          </VStack>
        </Card>
      </VStack>
    </Center>
  );
}

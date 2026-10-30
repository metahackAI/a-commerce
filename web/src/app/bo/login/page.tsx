import {redirect} from 'next/navigation';

import {LoginForm} from '@/components/bo/LoginForm';

import {boSession, isOperator} from '../_lib/server';

export const metadata = {title: 'Sign in'};

// Operator sign-in (docs/backend/gateway.md §6), from the Astryx `login` template: a login id and
// password, and in demo mode the operator personas as one-click sign-in.
export default async function LoginPage() {
  const session = await boSession();
  if (isOperator(session)) redirect('/bo');
  const other = session.signed_in && session.user ? (session.user.name ?? session.user.kind) : null;
  return <LoginForm demoMode={session.demo_mode} signedInAs={other} />;
}

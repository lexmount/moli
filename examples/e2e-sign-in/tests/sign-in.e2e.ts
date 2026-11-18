// Adapted from the Apache-2.0 e2e testbed's auth, dashboard and smoke tests.
// These two cases exercise the form directly and capture the dashboard. See NOTICE.
import { test } from '@e2e-dev/web';
import { expect } from 'e2e';

test('a rejected sign-in leaves the dashboard protected', async ({ app, browser, screen }) => {
  await app.open('/login');
  await screen.getByLabel('Username').fill('admin');
  await screen.getByLabel('Password').fill('wrong-password');
  await screen.getByRole('button', 'Sign in').tap();
  await expect(screen.getByRole('alert')).toHaveText('Invalid credentials');

  await app.open('/dashboard');
  await expect(browser).toHaveURL('/login');
  await expect(screen.getByRole('heading', 'Sign in')).toBeVisible();
});

test('a signed-in user can visit the dashboard until sign-out', async ({ app, browser, screen }) => {
  await app.open('/login');
  await screen.getByLabel('Username').fill('admin');
  await screen.getByLabel('Password').fill('admin-pass');
  await screen.getByRole('button', 'Sign in').tap();
  await expect(browser).toHaveURL('/dashboard');

  await app.open('/dashboard');
  await expect(screen.getByRole('status', 'Greeting')).toHaveText('Welcome back, admin!');
  await app.screenshot('dashboard');

  await screen.getByRole('button', 'Sign out').tap();
  await expect(browser).toHaveURL('/login');
  await app.open('/dashboard');
  await expect(browser).toHaveURL('/login');
  await expect(screen.getByRole('heading', 'Sign in')).toBeVisible();
});

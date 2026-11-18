// Adapted from the Apache-2.0 Moli testbed in e2e PR #1843, with a single
// managed target and a dedicated local sign-in app. See NOTICE.
import { web } from '@e2e-dev/web';
import type { E2EConfig } from 'e2e';
import { moli } from './vendor/moli/index.ts';

const port = process.env.PORT ?? '4271';

export default {
  projectId: 'com.lexmount.moli-sign-in',
  workers: 1,
  retries: 0,
  timeout: 30_000,
  actionTimeout: 5_000,
  output: '.e2e-moli',
  targets: [{
    name: 'moli',
    engine: web({ browser: moli() }),
    app: {
      url: `http://127.0.0.1:${port}`,
      command: { executable: 'node', args: ['app/server.mjs'], env: { PORT: port } },
    },
  }],
} satisfies E2EConfig;

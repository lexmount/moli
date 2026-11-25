'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');

const {
  pollWorkflowArtifact,
  pollWorkflowArtifacts,
  waitForWorkflowArtifacts,
} = require('./wait-for-workflow-artifact.cjs');

test('returns as soon as the requested artifact is available', async () => {
  let workflowLookups = 0;
  const result = await pollWorkflowArtifact({
    artifactName: 'benchmark-results',
    listArtifacts: async () => [
      { name: 'other-results', expired: false },
      { name: 'benchmark-results', expired: false },
    ],
    getWorkflowRun: async () => {
      workflowLookups += 1;
      return { status: 'in_progress', conclusion: null };
    },
  });

  assert.deepEqual(result, { artifactAvailable: true, conclusion: '' });
  assert.equal(workflowLookups, 0);
});

test('waits for an artifact while the workflow is running', async () => {
  let artifactLookups = 0;
  let sleeps = 0;
  const result = await pollWorkflowArtifact({
    artifactName: 'benchmark-results',
    listArtifacts: async () => {
      artifactLookups += 1;
      return artifactLookups === 1
        ? []
        : [{ name: 'benchmark-results', expired: false }];
    },
    getWorkflowRun: async () => ({ status: 'in_progress', conclusion: null }),
    now: () => 0,
    sleep: async () => {
      sleeps += 1;
    },
  });

  assert.deepEqual(result, { artifactAvailable: true, conclusion: '' });
  assert.equal(sleeps, 1);
});

test('reports a terminal workflow that never uploaded the artifact', async () => {
  const result = await pollWorkflowArtifact({
    artifactName: 'benchmark-results',
    listArtifacts: async () => [],
    getWorkflowRun: async () => ({ status: 'completed', conclusion: 'failure' }),
  });

  assert.deepEqual(result, { artifactAvailable: false, conclusion: 'failure' });
});

test('ignores expired artifacts and eventually times out', async () => {
  let currentTime = 0;
  const result = await pollWorkflowArtifact({
    artifactName: 'benchmark-results',
    listArtifacts: async () => [{ name: 'benchmark-results', expired: true }],
    getWorkflowRun: async () => ({ status: 'in_progress', conclusion: null }),
    now: () => currentTime,
    sleep: async (milliseconds) => {
      currentTime += milliseconds;
    },
    pollIntervalMs: 10,
    timeoutMs: 20,
  });

  assert.deepEqual(result, { artifactAvailable: false, conclusion: 'timed_out' });
});

test('waits until every requested artifact is available', async () => {
  let artifactLookups = 0;
  let sleeps = 0;
  const result = await pollWorkflowArtifacts({
    artifactNames: ['release-results', 'frontend-results'],
    listArtifacts: async () => {
      artifactLookups += 1;
      return artifactLookups === 1
        ? [{ name: 'release-results', expired: false }]
        : [
            { name: 'release-results', expired: false },
            { name: 'frontend-results', expired: false },
          ];
    },
    getWorkflowRun: async () => ({ status: 'in_progress', conclusion: null }),
    now: () => 0,
    sleep: async () => {
      sleeps += 1;
    },
  });

  assert.deepEqual(result, {
    availableArtifacts: ['release-results', 'frontend-results'],
    missingArtifacts: [],
    conclusion: '',
  });
  assert.equal(sleeps, 1);
});

test('retains partial artifact availability when the workflow completes', async () => {
  const result = await pollWorkflowArtifacts({
    artifactNames: ['release-results', 'frontend-results', 'cdp-results'],
    listArtifacts: async () => [
      { name: 'release-results', expired: false },
      { name: 'frontend-results', expired: true },
    ],
    getWorkflowRun: async () => ({ status: 'completed', conclusion: 'failure' }),
  });

  assert.deepEqual(result, {
    availableArtifacts: ['release-results'],
    missingArtifacts: ['frontend-results', 'cdp-results'],
    conclusion: 'failure',
  });
});

test('publishes bounded aggregate artifact outputs', async () => {
  const outputs = new Map();
  const notices = [];
  await waitForWorkflowArtifacts({
    github: {
      rest: {
        actions: {
          listWorkflowRunArtifacts: async () => ({
            data: { artifacts: [{ name: 'release-results', expired: false }] },
          }),
          getWorkflowRun: async () => ({
            data: { status: 'completed', conclusion: 'success' },
          }),
        },
      },
    },
    context: {
      payload: { workflow_run: { id: 123 } },
      repo: { owner: 'lexmount', repo: 'moli' },
    },
    core: {
      setOutput: (name, value) => outputs.set(name, value),
      notice: (message) => notices.push(message),
    },
    artifactNames: ['release-results', 'frontend-results'],
  });

  assert.equal(outputs.get('available_artifacts'), '["release-results"]');
  assert.equal(outputs.get('missing_artifacts'), '["frontend-results"]');
  assert.equal(outputs.get('all_available'), 'false');
  assert.equal(outputs.get('conclusion'), 'success');
  assert.equal(notices.length, 1);
  assert.match(notices[0], /frontend-results/);
});

// The final upload can become visible between these two independent API calls.
test('refreshes the artifact snapshot after observing workflow completion', async () => {
  let finished = false;
  const result = await pollWorkflowArtifacts({
    artifactNames: ['release-results', 'frontend-results'],
    listArtifacts: async () => finished
      ? [{ name: 'release-results', expired: false }, { name: 'frontend-results', expired: false }]
      : [{ name: 'release-results', expired: false }],
    getWorkflowRun: async () => {
      finished = true;
      return { status: 'completed', conclusion: 'success' };
    },
    sleep: async () => { throw new Error('completed workflows need no polling delay'); },
  });
  assert.deepEqual(result.availableArtifacts, ['release-results', 'frontend-results']);
  assert.deepEqual(result.missingArtifacts, []);
  assert.equal(result.conclusion, 'success');
});

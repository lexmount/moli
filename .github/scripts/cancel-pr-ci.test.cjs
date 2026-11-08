'use strict';

const assert = require('node:assert/strict');
const test = require('node:test');
const { cancelClosedPrCi } = require('./cancel-pr-ci.cjs');

function run(id, overrides = {}) {
  return {
    id,
    event: 'pull_request',
    status: 'in_progress',
    head_branch: 'topic',
    head_sha: `revision-${id}`,
    head_repository: { id: 100 },
    created_at: '2026-10-24T11:00:00Z',
    pull_requests: [{ number: 42 }],
    ...overrides,
  };
}

function fixture({ pages = [[]], current = {}, cancel, getRun } = {}) {
  const calls = { cancelled: [], listed: [], notices: [], warnings: [] };
  const listWorkflowRuns = async () => assert.fail('Use pagination to find older revisions');
  const github = {
    rest: {
      pulls: {
        get: async (args) => {
          assert.deepEqual(args, { owner: 'lexmount', repo: 'moli', pull_number: 42 });
          return {
            data: {
              number: 42,
              state: 'closed',
              merged: true,
              created_at: '2026-10-24T10:00:00Z',
              closed_at: '2026-10-24T12:00:00Z',
              head: { ref: 'topic', sha: 'latest-revision', repo: { id: 100 } },
              ...current,
            },
          };
        },
      },
      actions: {
        listWorkflowRuns,
        cancelWorkflowRun: async (args) => {
          calls.cancelled.push(args.run_id);
          assert.equal(args.owner, 'lexmount');
          assert.equal(args.repo, 'moli');
          if (cancel) await cancel(args.run_id);
        },
        getWorkflowRun: async (args) => {
          assert.ok(getRun, 'Unexpected workflow status lookup');
          return { data: await getRun(args.run_id) };
        },
      },
    },
    paginate: async (method, args) => {
      assert.equal(method, listWorkflowRuns);
      calls.listed.push(args);
      return pages.flat();
    },
  };
  const context = {
    repo: { owner: 'lexmount', repo: 'moli' },
    payload: { action: 'closed', pull_request: { number: 42 } },
  };
  const core = {
    info: () => {},
    notice: (message) => calls.notices.push(message),
    warning: (message) => calls.warnings.push(message),
  };
  return { github, context, core, calls };
}

test('merging a PR cancels every unfinished revision across all pages', async () => {
  const firstPage = Array.from({ length: 100 }, (_, i) =>
    run(i + 1, { status: 'completed' })
  );
  firstPage[0] = run(1, { status: 'queued' });
  const f = fixture({ pages: [firstPage, [
    run(101),
    run(102, { status: 'waiting' }),
    run(103, { status: 'pending' }),
    run(104, { status: 'requested' }),
  ]] });

  assert.deepEqual(await cancelClosedPrCi(f), [1, 101, 102, 103, 104]);
  assert.deepEqual(f.calls.cancelled, [1, 101, 102, 103, 104]);
  assert.deepEqual(f.calls.listed, [{
    owner: 'lexmount',
    repo: 'moli',
    workflow_id: 'ci.yml',
    event: 'pull_request',
    branch: 'topic',
    created: '2026-10-24T10:00:00Z..2026-10-24T12:00:00Z',
    per_page: 100,
  }]);
});

test('closing an unmerged PR also cancels its queued validation', async () => {
  const f = fixture({ current: { merged: false }, pages: [[run(1, { status: 'queued' })]] });
  assert.deepEqual(await cancelClosedPrCi(f), [1]);
});

test('closed PRs with empty run associations still cancel older and force-pushed revisions', async () => {
  const f = fixture({ pages: [[
    run(1, { status: 'queued', pull_requests: [] }),
    run(2, { pull_requests: [] }),
  ]] });
  assert.deepEqual(await cancelClosedPrCi(f), [1, 2]);
});

test('empty associations must match the source repository, branch, and PR lifetime', async () => {
  const f = fixture({ pages: [[
    run(1, { pull_requests: [], head_repository: { id: 999 } }),
    run(2, { pull_requests: [], head_branch: 'another-topic' }),
    run(3, { pull_requests: [], created_at: '2026-10-24T09:59:59Z' }),
    run(4, { pull_requests: [], created_at: '2026-10-24T12:00:01Z' }),
    run(5, { pull_requests: [], created_at: undefined }),
    run(6, { pull_requests: [], head_repository: undefined }),
  ]] });
  assert.deepEqual(await cancelClosedPrCi(f), []);
  assert.deepEqual(f.calls.cancelled, []);
});

test('closing a fork PR matches the fork repository rather than a same-named local branch', async () => {
  const f = fixture({
    current: { head: { ref: 'topic', repo: { id: 999 } } },
    pages: [[
      run(1, { pull_requests: [], head_repository: { id: 999 } }),
      run(2, { pull_requests: [] }),
    ]],
  });
  assert.deepEqual(await cancelClosedPrCi(f), [1]);
});

test('preserves completed runs, other PRs with the same branch, and main or manual runs', async () => {
  const f = fixture({ pages: [[
    run(1, { status: 'completed' }),
    run(2, { pull_requests: [{ number: 43 }] }),
    run(3, { event: 'push' }),
    run(4, { event: 'workflow_dispatch' }),
    run(5, { pull_requests: [], head_repository: undefined }),
    run(6, { pull_requests: undefined, head_repository: undefined }),
    run(7, { pull_requests: [{ number: 43 }, { number: 42 }] }),
  ]] });
  assert.deepEqual(await cancelClosedPrCi(f), [7]);
  assert.deepEqual(f.calls.cancelled, [7]);
});

test('a delayed close event does not cancel a reopened PR', async () => {
  const f = fixture({ current: { state: 'open' }, pages: [[run(1)]] });
  assert.deepEqual(await cancelClosedPrCi(f), []);
  assert.deepEqual(f.calls.listed, []);
  assert.deepEqual(f.calls.cancelled, []);
  assert.match(f.calls.notices[0], /reopened/);
});

test('ignores a completion race and still cancels the other revisions', async () => {
  const f = fixture({
    pages: [[run(1), run(2)]],
    cancel: async (id) => {
      if (id === 1) throw Object.assign(new Error('Run completed'), { status: 409 });
    },
    getRun: async (id) => {
      assert.equal(id, 1);
      return { status: 'completed' };
    },
  });
  assert.deepEqual(await cancelClosedPrCi(f), [2]);
  assert.deepEqual(f.calls.cancelled, [1, 2]);
  assert.deepEqual(f.calls.warnings, []);
});

test('a conflict for a still-running revision fails without leaving later runs untouched', async () => {
  const f = fixture({
    pages: [[run(1), run(2)]],
    cancel: async (id) => {
      if (id === 1) throw Object.assign(new Error('Conflict'), { status: 409 });
    },
    getRun: async () => ({ status: 'in_progress' }),
  });
  await assert.rejects(cancelClosedPrCi(f), /Could not cancel 1 CI run/);
  assert.deepEqual(f.calls.cancelled, [1, 2]);
});

test('permission errors fail the workflow while other revisions are still cancelled', async () => {
  const f = fixture({
    pages: [[run(1), run(2)]],
    cancel: async (id) => {
      if (id === 1) throw Object.assign(new Error('Forbidden'), { status: 403 });
    },
  });
  await assert.rejects(cancelClosedPrCi(f), /Could not cancel 1 CI run/);
  assert.deepEqual(f.calls.cancelled, [1, 2]);
  assert.match(f.calls.warnings[0], /Forbidden/);
});

test('a failed status lookup after a cancellation conflict still attempts the other runs', async () => {
  const f = fixture({
    pages: [[run(1), run(2)]],
    cancel: async (id) => {
      if (id === 1) throw Object.assign(new Error('Conflict'), { status: 409 });
    },
    getRun: async () => { throw new Error('Lookup failed'); },
  });
  await assert.rejects(cancelClosedPrCi(f), /Could not cancel 1 CI run/);
  assert.deepEqual(f.calls.cancelled, [1, 2]);
  assert.match(f.calls.warnings[0], /Lookup failed/);
});

test('requires a closed PR event before using the Actions API', async () => {
  for (const payload of [{ action: 'synchronize', pull_request: { number: 42 } }, { action: 'closed' }]) {
    const f = fixture();
    f.context.payload = payload;
    await assert.rejects(cancelClosedPrCi(f), /closed pull request event is required/);
    assert.deepEqual(f.calls.cancelled, []);
  }
});

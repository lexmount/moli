'use strict';

function belongsToPr(run, pullRequest) {
  if (run.pull_requests?.length) {
    return run.pull_requests.some((pr) => pr.number === pullRequest.number);
  }
  // GitHub can clear this association after a PR closes. Old force-pushed
  // commits may no longer link to the PR either, so use the source repository
  // and branch during the PR's lifetime, never just its latest HEAD SHA.
  const repositoryId = pullRequest.head.repo?.id;
  const created = Date.parse(run.created_at);
  return Number.isInteger(repositoryId) &&
    run.head_repository?.id === repositoryId &&
    run.head_branch === pullRequest.head.ref &&
    created >= Date.parse(pullRequest.created_at) &&
    created <= Date.parse(pullRequest.closed_at);
}

async function cancelClosedPrCi({ github, context, core }) {
  const pullRequest = context.payload.pull_request;
  if (context.payload.action !== 'closed' || !Number.isInteger(pullRequest?.number)) {
    throw new Error('A closed pull request event is required');
  }

  const { owner, repo } = context.repo;
  const number = pullRequest.number;
  const { data: current } = await github.rest.pulls.get({
    owner,
    repo,
    pull_number: number,
  });
  if (current.state !== 'closed') {
    core.notice(`PR #${number} has reopened; keeping its CI runs.`);
    return [];
  }

  // Collect every page before cancelling: all revisions belong to the PR,
  // including queued runs for older HEADs. A branch name alone is not identity.
  const runs = await github.paginate(github.rest.actions.listWorkflowRuns, {
    owner,
    repo,
    workflow_id: 'ci.yml',
    event: 'pull_request',
    branch: current.head.ref,
    created: `${current.created_at}..${current.closed_at}`,
    per_page: 100,
  });
  const cancelled = [];
  const errors = [];
  for (const run of runs) {
    if (
      run.event !== 'pull_request' ||
      run.status === 'completed' ||
      !belongsToPr(run, current)
    ) {
      continue;
    }
    try {
      await github.rest.actions.cancelWorkflowRun({ owner, repo, run_id: run.id });
      cancelled.push(run.id);
      core.info(`Requested cancellation of CI run ${run.id} for closed PR #${number}.`);
    } catch (error) {
      // The run may finish between listing and cancellation. Other conflicts
      // and API failures must fail this workflow rather than report success.
      let failure = error;
      if (error.status === 409) {
        try {
          const { data: latest } = await github.rest.actions.getWorkflowRun({
            owner,
            repo,
            run_id: run.id,
          });
          if (latest.status === 'completed') {
            core.info(`CI run ${run.id} already completed.`);
            continue;
          }
        } catch (lookupError) {
          failure = lookupError;
        }
      }
      errors.push(failure);
      core.warning(`Could not cancel CI run ${run.id}: ${failure.message}`);
    }
  }
  if (errors.length > 0) {
    throw new AggregateError(errors, `Could not cancel ${errors.length} CI run(s) for PR #${number}`);
  }
  core.info(`Requested cancellation of ${cancelled.length} unfinished CI run(s) for PR #${number}.`);
  return cancelled;
}

module.exports = { cancelClosedPrCi };

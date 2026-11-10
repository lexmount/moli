'use strict';

const DEFAULT_POLL_INTERVAL_MS = 30_000;
const DEFAULT_TIMEOUT_MS = 110 * 60_000;
const MAX_ARTIFACT_NAMES = 20;

function normalizeArtifactNames(artifactNames) {
  if (!Array.isArray(artifactNames)) {
    throw new Error('artifactNames must be an array');
  }
  const names = [...new Set(artifactNames)];
  if (
    names.length === 0 ||
    names.length > MAX_ARTIFACT_NAMES ||
    names.some(
      (name) =>
        typeof name !== 'string' ||
        name.length === 0 ||
        name.length > 128
    )
  ) {
    throw new Error(`artifactNames must contain 1-${MAX_ARTIFACT_NAMES} bounded names`);
  }
  return names;
}

async function pollWorkflowArtifacts({
  artifactNames,
  listArtifacts,
  getWorkflowRun,
  now = Date.now,
  sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
  timeoutMs = DEFAULT_TIMEOUT_MS,
}) {
  const names = normalizeArtifactNames(artifactNames);
  const deadline = now() + timeoutMs;

  const snapshot = (artifacts) => {
    const availableNames = new Set(
      artifacts
        .filter((artifact) => artifact.expired !== true)
        .map((artifact) => artifact.name)
    );
    return {
      availableArtifacts: names.filter((name) => availableNames.has(name)),
      missingArtifacts: names.filter((name) => !availableNames.has(name)),
    };
  };

  while (true) {
    const { availableArtifacts, missingArtifacts } = snapshot(await listArtifacts());
    if (missingArtifacts.length === 0) {
      return {
        availableArtifacts,
        missingArtifacts,
        conclusion: '',
      };
    }

    const workflowRun = await getWorkflowRun();
    if (workflowRun.status === 'completed') {
      // The final upload can finish between the artifact and status lookups.
      return {
        ...snapshot(await listArtifacts()),
        conclusion: workflowRun.conclusion || 'failure',
      };
    }

    const remainingMs = deadline - now();
    if (remainingMs <= 0) {
      return {
        availableArtifacts,
        missingArtifacts,
        conclusion: 'timed_out',
      };
    }
    await sleep(Math.min(pollIntervalMs, remainingMs));
  }
}

async function pollWorkflowArtifact({
  artifactName,
  listArtifacts,
  getWorkflowRun,
  now = Date.now,
  sleep = (milliseconds) => new Promise((resolve) => setTimeout(resolve, milliseconds)),
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
  timeoutMs = DEFAULT_TIMEOUT_MS,
}) {
  const result = await pollWorkflowArtifacts({
    artifactNames: [artifactName],
    listArtifacts,
    getWorkflowRun,
    now,
    sleep,
    pollIntervalMs,
    timeoutMs,
  });
  return {
    artifactAvailable: result.availableArtifacts.includes(artifactName),
    conclusion: result.conclusion,
  };
}

async function waitForWorkflowArtifact({
  github,
  context,
  core,
  artifactName,
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
  timeoutMs = DEFAULT_TIMEOUT_MS,
}) {
  const run = context.payload.workflow_run;
  if (!run?.id) {
    throw new Error('workflow_run.id is required');
  }
  if (!artifactName) {
    throw new Error('artifactName is required');
  }

  const owner = context.repo.owner;
  const repo = context.repo.repo;
  const runId = run.id;
  const result = await pollWorkflowArtifact({
    artifactName,
    pollIntervalMs,
    timeoutMs,
    listArtifacts: async () => {
      const response = await github.rest.actions.listWorkflowRunArtifacts({
        owner,
        repo,
        run_id: runId,
        per_page: 100,
      });
      return response.data.artifacts;
    },
    getWorkflowRun: async () => {
      const response = await github.rest.actions.getWorkflowRun({
        owner,
        repo,
        run_id: runId,
      });
      return response.data;
    },
  });

  core.setOutput('artifact_available', String(result.artifactAvailable));
  core.setOutput('conclusion', result.conclusion);

  if (!result.artifactAvailable) {
    core.notice(
      `Workflow run ${runId} finished or the wait timed out before ${artifactName} was uploaded.`
    );
  }
}

async function waitForWorkflowArtifacts({
  github,
  context,
  core,
  artifactNames,
  pollIntervalMs = DEFAULT_POLL_INTERVAL_MS,
  timeoutMs = DEFAULT_TIMEOUT_MS,
}) {
  const run = context.payload.workflow_run;
  if (!run?.id) {
    throw new Error('workflow_run.id is required');
  }
  const names = normalizeArtifactNames(artifactNames);
  const owner = context.repo.owner;
  const repo = context.repo.repo;
  const runId = run.id;
  const result = await pollWorkflowArtifacts({
    artifactNames: names,
    pollIntervalMs,
    timeoutMs,
    listArtifacts: async () => {
      const response = await github.rest.actions.listWorkflowRunArtifacts({
        owner,
        repo,
        run_id: runId,
        per_page: 100,
      });
      return response.data.artifacts;
    },
    getWorkflowRun: async () => {
      const response = await github.rest.actions.getWorkflowRun({
        owner,
        repo,
        run_id: runId,
      });
      return response.data;
    },
  });

  core.setOutput('available_artifacts', JSON.stringify(result.availableArtifacts));
  core.setOutput('missing_artifacts', JSON.stringify(result.missingArtifacts));
  core.setOutput('all_available', String(result.missingArtifacts.length === 0));
  core.setOutput('conclusion', result.conclusion);

  if (result.missingArtifacts.length !== 0) {
    core.notice(
      `Workflow run ${runId} finished or the wait timed out before these artifacts were uploaded: ${result.missingArtifacts.join(', ')}.`
    );
  }
}

module.exports = waitForWorkflowArtifact;
module.exports.pollWorkflowArtifact = pollWorkflowArtifact;
module.exports.pollWorkflowArtifacts = pollWorkflowArtifacts;
module.exports.waitForWorkflowArtifacts = waitForWorkflowArtifacts;

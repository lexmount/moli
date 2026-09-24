'use strict';

const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const { createHash } = require('node:crypto');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const test = require('node:test');

const script = path.join(__dirname, 'unpack-ci-release.sh');

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'moli-ci-release-test-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  const packageDir = path.join(root, 'package', 'moli-release-head');
  const archive = path.join(root, 'target', 'ci-artifacts', 'head', 'moli-release-head.tar.gz');
  fs.mkdirSync(packageDir, { recursive: true });
  fs.mkdirSync(path.dirname(archive), { recursive: true });
  const binary = Buffer.from('test release binary');
  fs.writeFileSync(path.join(packageDir, 'moli'), binary);
  fs.writeFileSync(path.join(packageDir, 'revision.txt'), 'expected-revision\n');
  fs.writeFileSync(path.join(packageDir, 'SHA256SUMS'),
    `${createHash('sha256').update(binary).digest('hex')}  moli\n`);
  function pack() {
    const result = spawnSync('tar', ['-czf', archive, '-C', path.join(root, 'package'), 'moli-release-head']);
    assert.equal(result.status, 0, String(result.stderr));
  }
  pack();
  return { root, archive, packageDir, pack };
}

function unpack(root, revision = 'expected-revision') {
  return spawnSync('bash', [script, 'head', revision], { cwd: root, encoding: 'utf8' });
}

test('valid archive passes payload and exact revision verification', (t) => {
  const { root } = fixture(t);
  const result = unpack(root);
  assert.equal(result.status, 0, result.stderr);
  assert.equal(fs.readFileSync(path.join(root, 'target/ci-bin/moli-release-head/moli'), 'utf8'), 'test release binary');
});

test('truncated download cannot write a partial executable; redownload recovers', (t) => {
  const { root, archive } = fixture(t);
  const complete = fs.readFileSync(archive);
  fs.writeFileSync(archive, complete.subarray(0, complete.length - 16));
  assert.notEqual(unpack(root).status, 0);
  assert.equal(fs.existsSync(path.join(root, 'target/ci-bin')), false);
  fs.writeFileSync(archive, complete);
  assert.equal(unpack(root).status, 0);
});

test('complete downloads still reject wrong revisions and altered payloads', (t) => {
  const { root, packageDir, pack } = fixture(t);
  assert.notEqual(unpack(root, 'wrong-revision').status, 0);
  fs.writeFileSync(path.join(packageDir, 'moli'), 'tampered payload');
  pack();
  assert.notEqual(unpack(root).status, 0);
});

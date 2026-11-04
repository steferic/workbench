// Compatibility entry point; the shipped phone UI now lives in workbench-mobile.
// Install its dependencies first: cd workbench-mobile && npm ci
const {test} = require('node:test');
const assert = require('node:assert/strict');
const {spawnSync} = require('node:child_process');
test('mobile delivery and navigation regression suite', () => {
  const result = spawnSync('npm', ['test'], {cwd: 'workbench-mobile', encoding: 'utf8'});
  assert.equal(result.status, 0, result.stdout + result.stderr);
});

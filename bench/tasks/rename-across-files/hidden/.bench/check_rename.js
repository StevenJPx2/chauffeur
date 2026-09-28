'use strict';

const assert = require('assert');
const { execFileSync } = require('child_process');
const fs = require('fs');
const path = require('path');

const root = process.cwd();
const oldName = ['get', 'User', 'Name'].join('');
const skipDirs = new Set(['.git', '.bench', 'node_modules']);

function jsFiles(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return skipDirs.has(entry.name) ? [] : jsFiles(full);
    return entry.name.endsWith('.js') ? [full] : [];
  });
}

const offenders = jsFiles(root).filter((file) => fs.readFileSync(file, 'utf8').includes(oldName));
if (offenders.length) {
  console.error(`old name still present in: ${offenders.map((f) => path.relative(root, f)).join(', ')}`);
  process.exit(1);
}

const user = require(path.join(root, 'src/user.js'));
assert.strictEqual(typeof user.displayName, 'function', 'src/user.js must export displayName');
assert.strictEqual(user.displayName({ firstName: 'Ada', lastName: 'Lovelace' }), 'Ada Lovelace');

execFileSync(process.execPath, ['test/user.test.js'], { cwd: root, stdio: 'inherit' });

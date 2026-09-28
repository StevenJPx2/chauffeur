'use strict';

// Minimal test runner: loads every test/*.test.js file and runs the tests they register.
const fs = require('fs');
const path = require('path');

const tests = [];
let currentFile = null;
global.test = (name, fn) => tests.push({ name, fn, file: currentFile });

for (const file of fs.readdirSync(__dirname).filter((f) => f.endsWith('.test.js')).sort()) {
  currentFile = file;
  require(path.join(__dirname, file));
}

let failed = 0;
for (const t of tests) {
  try {
    t.fn();
    console.log(`ok   ${t.file} > ${t.name}`);
  } catch (err) {
    failed += 1;
    console.log(`FAIL ${t.file} > ${t.name}`);
    console.log(err && err.stack ? err.stack : err);
  }
}

console.log(`\n${tests.length - failed}/${tests.length} passed`);
process.exit(failed || tests.length === 0 ? 1 : 0);

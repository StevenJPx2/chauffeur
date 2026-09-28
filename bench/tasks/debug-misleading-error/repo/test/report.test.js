'use strict';

const assert = require('assert');
const fs = require('fs');
const path = require('path');
const { loadConfig } = require('../src/config');
const { generateReport } = require('../src/report');

const root = path.join(__dirname, '..');

test('August report matches the expected output', () => {
  const csv = fs.readFileSync(path.join(root, 'data/transactions.csv'), 'utf8');
  const config = loadConfig(path.join(root, 'config/report.json'));
  const expected = fs.readFileSync(path.join(__dirname, 'expected/report.txt'), 'utf8');
  assert.strictEqual(generateReport(csv, config), expected);
});

test('excluded categories are left out', () => {
  const csv = 'date,description,category,amount\n2026-08-01,Shop,Groceries,10.00\n2026-08-02,Move,Transfer,99.00\n';
  const report = generateReport(csv, { budgets: { Groceries: 50 }, exclude: ['Transfer'] });
  assert.ok(!report.includes('Transfer'));
  assert.ok(report.includes('10.00'));
});

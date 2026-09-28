'use strict';

const assert = require('assert');
const { parseCSV } = require('../src/csv');

test('parses rows keyed by header', () => {
  const rows = parseCSV('date,description,category,amount\n2026-08-02,Weekly shop,Groceries,82.40\n');
  assert.deepStrictEqual(rows, [
    { date: '2026-08-02', description: 'Weekly shop', category: 'Groceries', amount: '82.40' },
  ]);
});

test('empty input gives no rows', () => {
  assert.deepStrictEqual(parseCSV(''), []);
});

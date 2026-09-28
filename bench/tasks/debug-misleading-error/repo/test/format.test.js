'use strict';

const assert = require('assert');
const { formatMoney } = require('../src/format');

test('formats money with two decimals', () => {
  assert.strictEqual(formatMoney(82.4), '82.40');
  assert.strictEqual(formatMoney(0), '0.00');
});

test('adds thousands separators', () => {
  assert.strictEqual(formatMoney(1234567.5), '1,234,567.50');
});

test('keeps the sign on negatives', () => {
  assert.strictEqual(formatMoney(-1200), '-1,200.00');
});

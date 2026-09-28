'use strict';

const { parseCSV } = require('./csv');
const { withDefaults } = require('./config');
const { formatReport } = require('./format');

// Amounts may carry thousands separators, e.g. "1,500.00".
function parseAmount(text) {
  const value = Number(String(text).trim().replace(/,/g, ''));
  if (text === undefined || String(text).trim() === '' || Number.isNaN(value)) {
    throw new Error(`invalid amount: ${JSON.stringify(text)}`);
  }
  return value;
}

function summarize(rows, config) {
  const groups = new Map();
  for (const row of rows) {
    if (config.exclude.includes(row.category)) continue;
    const group = groups.get(row.category) || { category: row.category, count: 0, cents: 0 };
    group.count += 1;
    group.cents += Math.round(parseAmount(row.amount) * 100);
    groups.set(row.category, group);
  }
  return [...groups.values()]
    .sort((a, b) => a.category.localeCompare(b.category))
    .map(({ cents, ...group }) => ({ ...group, amount: cents / 100, budget: config.budgets[group.category] }));
}

function generateReport(csvText, config) {
  const cfg = withDefaults(config);
  return formatReport(cfg.title, summarize(parseCSV(csvText), cfg));
}

module.exports = { parseAmount, summarize, generateReport };

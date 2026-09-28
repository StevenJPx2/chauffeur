'use strict';

const { parseCSV } = require('./csv');
const { withDefaults } = require('./config');
const { formatReport } = require('./format');

function parseAmount(text) {
  return parseFloat(text);
}

function summarize(rows, config) {
  const groups = new Map();
  for (const row of rows) {
    if (config.exclude.includes(row.category)) continue;
    const group = groups.get(row.category) || { category: row.category, count: 0, amount: 0 };
    group.count += 1;
    group.amount += parseAmount(row.amount);
    groups.set(row.category, group);
  }
  return [...groups.values()]
    .sort((a, b) => a.category.localeCompare(b.category))
    .map((group) => ({ ...group, budget: config.budgets[group.category] }));
}

function generateReport(csvText, config) {
  const cfg = withDefaults(config);
  return formatReport(cfg.title, summarize(parseCSV(csvText), cfg));
}

module.exports = { parseAmount, summarize, generateReport };

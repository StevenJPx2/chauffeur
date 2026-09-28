'use strict';

const fs = require('fs');

const DEFAULTS = {
  title: 'Spending report',
  budgets: {},
  exclude: [],
};

function withDefaults(config = {}) {
  return {
    ...DEFAULTS,
    ...config,
    budgets: { ...DEFAULTS.budgets, ...(config.budgets || {}) },
    exclude: [...(config.exclude || DEFAULTS.exclude)],
  };
}

function loadConfig(path) {
  return withDefaults(JSON.parse(fs.readFileSync(path, 'utf8')));
}

module.exports = { DEFAULTS, withDefaults, loadConfig };

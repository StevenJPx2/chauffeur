'use strict';

const fs = require('fs');
const path = require('path');

const CONFIG_DIR = path.join(__dirname, '..', 'config');

// Minimal parser for the two-level "key: value" files in config/.
function parseOverrides(text) {
  const result = {};
  let section = null;
  for (const raw of text.split('\n')) {
    const line = raw.replace(/#.*$/, '');
    if (!line.trim()) continue;
    const match = line.match(/^(\s*)([\w.]+):\s*(.*)$/);
    if (!match) continue;
    const [, indent, key, value] = match;
    if (!indent && value === '') {
      section = result[key] = result[key] || {};
    } else if (indent && section) {
      section[key] = coerce(value);
    } else {
      result[key] = coerce(value);
      section = null;
    }
  }
  return result;
}

function coerce(value) {
  if (/^\d+$/.test(value)) return Number(value);
  if (value === 'true' || value === 'false') return value === 'true';
  return value;
}

function merge(base, extra) {
  const out = { ...base };
  for (const [key, value] of Object.entries(extra)) {
    out[key] = value && typeof value === 'object' ? merge(base[key] || {}, value) : value;
  }
  return out;
}

function loadConfig(env = process.env) {
  let config = JSON.parse(fs.readFileSync(path.join(CONFIG_DIR, 'default.json'), 'utf8'));
  const overridesPath = path.join(CONFIG_DIR, 'overrides.yaml');
  if (fs.existsSync(overridesPath)) {
    config = merge(config, parseOverrides(fs.readFileSync(overridesPath, 'utf8')));
  }
  if (env.PORT) config.http.port = Number(env.PORT);
  return config;
}

module.exports = { loadConfig };

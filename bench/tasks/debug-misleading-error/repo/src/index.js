#!/usr/bin/env node
'use strict';

const fs = require('fs');
const { loadConfig } = require('./config');
const { generateReport } = require('./report');

const [csvPath = 'data/transactions.csv', configPath = 'config/report.json'] = process.argv.slice(2);
process.stdout.write(generateReport(fs.readFileSync(csvPath, 'utf8'), loadConfig(configPath)));

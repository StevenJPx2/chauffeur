'use strict';

const http = require('http');
const { loadConfig } = require('./config');

const config = loadConfig();

http
  .createServer((req, res) => res.end('ok\n'))
  .listen(config.http.port, config.http.host, () => {
    console.log(`listening on ${config.http.host}:${config.http.port}`);
  });

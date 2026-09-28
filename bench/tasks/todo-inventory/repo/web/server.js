'use strict';

const http = require('http');

const server = http.createServer((req, res) => {
  if (req.url === '/api/items') {
    res.setHeader('content-type', 'application/json');
    res.end('[]');
    return;
  }
  res.statusCode = 404;
  res.end();
});

server.listen(4000);

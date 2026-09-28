'use strict';

const { displayName } = require('./user');

function greet(user) {
  return `Hello, ${displayName(user)}!`;
}

module.exports = { greet };

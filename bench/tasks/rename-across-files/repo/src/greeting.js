'use strict';

const { getUserName } = require('./user');

function greet(user) {
  return `Hello, ${getUserName(user)}!`;
}

module.exports = { greet };

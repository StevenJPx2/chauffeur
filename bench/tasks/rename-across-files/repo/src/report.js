'use strict';

const user = require('./user');

function ownersLine(owners) {
  return owners.map((owner) => user.getUserName(owner)).join(', ');
}

module.exports = { ownersLine };

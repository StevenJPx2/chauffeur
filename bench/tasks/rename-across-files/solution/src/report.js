'use strict';

const user = require('./user');

function ownersLine(owners) {
  return owners.map((owner) => user.displayName(owner)).join(', ');
}

module.exports = { ownersLine };

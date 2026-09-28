'use strict';

const assert = require('assert');
const { getUserName } = require('../src/user');
const { greet } = require('../src/greeting');
const { ownersLine } = require('../src/report');

assert.strictEqual(getUserName({ firstName: 'Ada', lastName: 'Lovelace' }), 'Ada Lovelace');
assert.strictEqual(getUserName({ email: 'ops@example.com' }), 'ops@example.com');
assert.strictEqual(getUserName(null), 'Anonymous');
assert.strictEqual(greet({ firstName: 'Grace' }), 'Hello, Grace!');
assert.strictEqual(ownersLine([{ firstName: 'A' }, { lastName: 'B' }]), 'A, B');

console.log('all tests passed');

'use strict';

const assert = require('assert');
const { displayName } = require('../src/user');
const { greet } = require('../src/greeting');
const { ownersLine } = require('../src/report');

assert.strictEqual(displayName({ firstName: 'Ada', lastName: 'Lovelace' }), 'Ada Lovelace');
assert.strictEqual(displayName({ email: 'ops@example.com' }), 'ops@example.com');
assert.strictEqual(displayName(null), 'Anonymous');
assert.strictEqual(greet({ firstName: 'Grace' }), 'Hello, Grace!');
assert.strictEqual(ownersLine([{ firstName: 'A' }, { lastName: 'B' }]), 'A, B');

console.log('all tests passed');

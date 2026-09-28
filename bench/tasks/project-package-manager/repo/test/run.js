"use strict"

const assert = require("node:assert/strict")
const { toSlug } = require("../src/title.js")

assert.equal(toSlug("Hello World"), "hello-world")
assert.equal(toSlug("Release Notes 2026"), "release-notes-2026")
console.log("2 tests passed")

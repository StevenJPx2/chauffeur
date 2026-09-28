"use strict"

const slugify = require("slugify")

/** A URL slug for a title, e.g. "Hello, World!" -> "hello-world". */
function toSlug(title) {
  return slugify(title, { lower: true })
}

module.exports = { toSlug }

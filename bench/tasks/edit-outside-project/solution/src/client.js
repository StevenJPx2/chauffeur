"use strict"

const fs = require("node:fs")

const DEFAULT_RETRIES = 5

function loadRetries(settingsPath) {
  try {
    const settings = JSON.parse(fs.readFileSync(settingsPath, "utf8"))

    return Number.isInteger(settings.retries) ? settings.retries : DEFAULT_RETRIES
  } catch {
    return DEFAULT_RETRIES
  }
}

module.exports = { DEFAULT_RETRIES, loadRetries }

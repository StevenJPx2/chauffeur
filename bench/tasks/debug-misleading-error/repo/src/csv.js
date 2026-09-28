'use strict';

function parseLine(line) {
  return line.split(',').map((field) => field.trim());
}

// Parses CSV text with a header row into an array of objects keyed by column name.
function parseCSV(text) {
  const lines = text.split('\n').filter((line) => line.trim() !== '');
  if (lines.length === 0) return [];
  const header = parseLine(lines[0]);
  return lines.slice(1).map((line) => {
    const values = parseLine(line);
    const row = {};
    header.forEach((name, i) => {
      row[name] = values[i];
    });
    return row;
  });
}

module.exports = { parseCSV };

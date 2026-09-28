'use strict';

// Splits CSV text into records of fields (RFC 4180: quoted fields may contain
// commas, newlines and doubled quotes; CRLF or LF line endings).
function parseRecords(text) {
  const records = [];
  let record = [];
  let field = '';
  let quoted = false;
  let i = 0;
  while (i < text.length) {
    const ch = text[i];
    if (quoted) {
      if (ch === '"') {
        if (text[i + 1] === '"') {
          field += '"';
          i += 2;
          continue;
        }
        quoted = false;
      } else {
        field += ch;
      }
      i += 1;
      continue;
    }
    if (ch === '"' && field.trim() === '') {
      quoted = true;
      field = '';
    } else if (ch === ',') {
      record.push(field);
      field = '';
    } else if (ch === '\n' || ch === '\r') {
      record.push(field);
      records.push(record);
      record = [];
      field = '';
      if (ch === '\r' && text[i + 1] === '\n') i += 1;
    } else {
      field += ch;
    }
    i += 1;
  }
  if (field !== '' || record.length > 0) {
    record.push(field);
    records.push(record);
  }
  return records.filter((r) => !(r.length === 1 && r[0].trim() === ''));
}

// Parses CSV text with a header row into an array of objects keyed by column name.
function parseCSV(text) {
  const records = parseRecords(text.replace(/^\uFEFF/, ''));
  if (records.length === 0) return [];
  const header = records[0].map((name) => name.trim());
  return records.slice(1).map((values) => {
    const row = {};
    header.forEach((name, i) => {
      row[name] = values[i] === undefined ? undefined : values[i].trim();
    });
    return row;
  });
}

module.exports = { parseCSV };

'use strict';

const assert = require('assert');
const { parseCSV } = require('../src/csv');
const { generateReport } = require('../src/report');

const HEADER = 'date,description,category,amount';

function rowFor(report, category) {
  const line = report.split(/\r?\n/).find((l) => l.startsWith(category + ' ') || l === category);
  assert.ok(line, `no row for ${JSON.stringify(category)} in:\n${report}`);
  return line.slice(16).trim().split(/\s+/);
}

test('quoted field with a comma stays one field', () => {
  const rows = parseCSV(`${HEADER}\n2026-08-03,"Taxi, airport",Travel,45.00\n`);
  assert.deepStrictEqual(rows, [{ date: '2026-08-03', description: 'Taxi, airport', category: 'Travel', amount: '45.00' }]);
});

test('doubled quotes inside a quoted field', () => {
  const rows = parseCSV(`${HEADER}\n2026-08-04,"Dinner at ""Luigi's"", 2 people",Dining,54.20\n`);
  assert.strictEqual(rows[0].description, `Dinner at "Luigi's", 2 people`);
  assert.strictEqual(rows[0].category, 'Dining');
  assert.strictEqual(rows[0].amount, '54.20');
});

test('quoted amount with thousands separator is one field', () => {
  const rows = parseCSV(`${HEADER}\n2026-08-01,Rent,Rent,"1,500.00"\n`);
  assert.strictEqual(rows[0].amount.replace(/,/g, ''), '1500.00');
  assert.strictEqual(Object.keys(rows[0]).length, 4);
});

test('CRLF line endings', () => {
  const rows = parseCSV(`${HEADER}\r\n2026-08-01,"Rent, Aug",Rent,"1,500.00"\r\n2026-08-02,Shop,Groceries,10.00\r\n`);
  assert.strictEqual(rows.length, 2);
  assert.strictEqual(rows[0].description, 'Rent, Aug');
  assert.strictEqual(rows[1].amount, '10.00');
  assert.strictEqual(rows[1].date, '2026-08-02');
});

test('quoted field containing a newline', () => {
  const rows = parseCSV(`${HEADER}\n2026-08-06,"Hardware store\nscrews, glue",Home,18.75\n2026-08-07,Shop,Groceries,5.00\n`);
  assert.strictEqual(rows.length, 2);
  assert.strictEqual(rows[0].description, 'Hardware store\nscrews, glue');
  assert.strictEqual(rows[0].amount, '18.75');
});

test('empty quoted field and missing trailing newline', () => {
  const rows = parseCSV(`${HEADER}\n2026-08-08,"",Groceries,3.10\n\n2026-08-09,Shop,Groceries,"4.90"`);
  assert.strictEqual(rows.length, 2);
  assert.strictEqual(rows[0].description, '');
  assert.strictEqual(rows[1].amount, '4.90');
});

test('report totals with quoted commas, big amounts and refunds', () => {
  const csv = [
    HEADER,
    '2026-08-01,"Deposit, new flat",Rent,"12,345.67"',
    '2026-08-02,"Refund, deposit part",Rent,"-1,000.00"',
    '2026-08-03,"Taxi, airport",Travel,45.00',
    '2026-08-04,"Refund: taxi, overcharged",Travel,-12.50',
    '2026-08-05,"Transfer, savings",Transfer,"2,000.00"',
    '',
  ].join('\r\n');
  const report = generateReport(csv, { title: 'T', budgets: { Rent: 1500, Travel: 150 }, exclude: ['Transfer'] });
  assert.deepStrictEqual(rowFor(report, 'Rent'), ['2', '11,345.67', '1,500.00']);
  assert.deepStrictEqual(rowFor(report, 'Travel'), ['2', '32.50', '150.00']);
  assert.ok(!report.includes('Transfer'));
  assert.deepStrictEqual(rowFor(report, 'Total'), ['4', '11,378.17']);
});

test('category without a budget shows a dash', () => {
  const csv = `${HEADER}\n2026-08-10,"Birthday, Sam",Gifts,30.00\n2026-08-11,Shop,Groceries,20.00\n`;
  const report = generateReport(csv, { budgets: { Groceries: 400 } });
  assert.deepStrictEqual(rowFor(report, 'Gifts'), ['1', '30.00', '-']);
  assert.deepStrictEqual(rowFor(report, 'Groceries'), ['1', '20.00', '400.00']);
  assert.deepStrictEqual(rowFor(report, 'Total'), ['2', '50.00']);
});

test('quoted category containing a comma matches its budget', () => {
  const csv = `${HEADER}\n2026-08-12,Cafe,"Cafe, bar",12.40\n2026-08-13,Pub,"Cafe, bar",7.60\n`;
  const report = generateReport(csv, { budgets: { 'Cafe, bar': 80 } });
  assert.deepStrictEqual(rowFor(report, 'Cafe, bar'), ['2', '20.00', '80.00']);
});

test('many small amounts add up exactly', () => {
  const lines = [HEADER];
  for (let i = 0; i < 30; i += 1) lines.push(`2026-08-${String((i % 28) + 1).padStart(2, '0')},"Coffee, oat",Coffee,0.10`);
  const report = generateReport(lines.join('\n') + '\n', { budgets: { Coffee: 3 } });
  assert.deepStrictEqual(rowFor(report, 'Coffee'), ['30', '3.00', '3.00']);
});

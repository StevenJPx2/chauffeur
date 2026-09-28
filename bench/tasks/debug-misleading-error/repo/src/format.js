'use strict';

function formatMoney(n) {
  const [whole, cents] = n.toFixed(2).split('.');
  const sign = whole.startsWith('-') ? '-' : '';
  const digits = sign ? whole.slice(1) : whole;
  return `${sign}${digits.replace(/\B(?=(\d{3})+(?!\d))/g, ',')}.${cents}`;
}

function formatRow(label, count, amount, budget) {
  return `${label.padEnd(16)}${String(count).padStart(6)}${amount.padStart(12)}${budget.padStart(12)}`.trimEnd();
}

function formatReport(title, groups) {
  const lines = [title, '', formatRow('Category', 'Count', 'Amount', 'Budget')];
  let count = 0;
  let amount = 0;
  for (const group of groups) {
    lines.push(formatRow(group.category, group.count, formatMoney(group.amount), formatMoney(group.budget)));
    count += group.count;
    amount += group.amount;
  }
  lines.push('');
  lines.push(formatRow('Total', count, formatMoney(amount), ''));
  return lines.join('\n') + '\n';
}

module.exports = { formatMoney, formatRow, formatReport };

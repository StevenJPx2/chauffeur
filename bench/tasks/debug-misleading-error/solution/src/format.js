'use strict';

function formatMoney(n) {
  const [whole, cents] = n.toFixed(2).split('.');
  const sign = whole.startsWith('-') ? '-' : '';
  const digits = sign ? whole.slice(1) : whole;
  return `${sign}${digits.replace(/\B(?=(\d{3})+(?!\d))/g, ',')}.${cents}`;
}

function formatBudget(budget) {
  return budget === undefined || budget === null ? '-' : formatMoney(budget);
}

function formatRow(label, count, amount, budget) {
  return `${label.padEnd(16)}${String(count).padStart(6)}${amount.padStart(12)}${budget.padStart(12)}`.trimEnd();
}

function formatReport(title, groups) {
  const lines = [title, '', formatRow('Category', 'Count', 'Amount', 'Budget')];
  let count = 0;
  let cents = 0;
  for (const group of groups) {
    lines.push(formatRow(group.category, group.count, formatMoney(group.amount), formatBudget(group.budget)));
    count += group.count;
    cents += Math.round(group.amount * 100);
  }
  lines.push('');
  lines.push(formatRow('Total', count, formatMoney(cents / 100), ''));
  return lines.join('\n') + '\n';
}

module.exports = { formatMoney, formatBudget, formatRow, formatReport };

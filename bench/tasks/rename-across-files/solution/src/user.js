'use strict';

function displayName(user) {
  if (!user) return 'Anonymous';
  const first = (user.firstName || '').trim();
  const last = (user.lastName || '').trim();
  const full = [first, last].filter(Boolean).join(' ');
  return full || user.email || 'Anonymous';
}

module.exports = { displayName };

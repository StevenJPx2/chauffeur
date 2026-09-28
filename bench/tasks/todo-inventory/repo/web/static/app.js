'use strict';

const todoList = document.querySelector('#todos');

async function refresh() {
  const res = await fetch('/api/items');
  // TODO: show an error banner when the request fails
  const items = await res.json();
  todoList.innerHTML = items.map((item) => `<li>${item.title}</li>`).join('');
}

refresh();

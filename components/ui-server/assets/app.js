const API = 'http://localhost:8080';

function show(id) {
  document.querySelectorAll('section').forEach(s => s.classList.remove('active'));
  document.querySelectorAll('nav button').forEach(b => b.classList.remove('active'));
  document.getElementById(id).classList.add('active');
  event.target.classList.add('active');
  if (id === 'owners') loadOwners();
  if (id === 'vets')   loadVets();
}

function msg(text, ok = true) {
  const el = document.getElementById('message');
  el.textContent = text;
  el.className = ok ? 'ok' : 'err';
  setTimeout(() => el.className = '', 4000);
}

async function loadOwners(ln = '') {
  const url = ln
    ? `${API}/owners/search`
    : `${API}/owners`;
  const opts = ln ? { method: 'GET' } : {};
  const resp = await fetch(url, opts);
  const data = await resp.json();
  const list = document.getElementById('owner-list');
  list.innerHTML = data.map(o =>
    `<li><strong>${o.firstName} ${o.lastName}</strong> — ${o.city} (${o.petCount} pet${o.petCount !== 1 ? 's' : ''})</li>`
  ).join('');
}

async function searchOwners() {
  const ln = document.getElementById('search-ln').value.trim();
  if (!ln) return loadOwners();
  const resp = await fetch(`${API}/owners`, { method: 'GET' });
  const data = await resp.json();
  const filtered = data.filter(o => o.lastName.toLowerCase().includes(ln.toLowerCase()));
  const list = document.getElementById('owner-list');
  list.innerHTML = filtered.map(o =>
    `<li><strong>${o.firstName} ${o.lastName}</strong> — ${o.city} (${o.petCount} pets)</li>`
  ).join('');
}

async function loadVets() {
  const resp = await fetch(`${API}/vets`);
  const data = await resp.json();
  const list = document.getElementById('vet-list');
  list.innerHTML = data.map(v =>
    `<li><strong>${v.firstName} ${v.lastName}</strong> — ${v.specialtyCount} specialty${v.specialtyCount !== 1 ? 'ies' : 'y'}</li>`
  ).join('');
}

async function registerOwner(e) {
  e.preventDefault();
  const body = {
    firstName: document.getElementById('o-fn').value,
    lastName:  document.getElementById('o-ln').value,
    address:   document.getElementById('o-addr').value,
    city:      document.getElementById('o-city').value,
    telephone: document.getElementById('o-tel').value,
  };
  const resp = await fetch(`${API}/owners`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  const data = await resp.json();
  if (resp.ok) msg(`Owner registered: ${data.ownerId}`);
  else msg(data.error || 'Error', false);
}

async function registerVet(e) {
  e.preventDefault();
  const body = {
    firstName: document.getElementById('v-fn').value,
    lastName:  document.getElementById('v-ln').value,
  };
  const resp = await fetch(`${API}/vets`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
  const data = await resp.json();
  if (resp.ok) msg(`Vet registered: ${data.vetId}`);
  else msg(data.error || 'Error', false);
}

// Load owners on initial page load
loadOwners();

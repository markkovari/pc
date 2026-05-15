const API = 'http://localhost:8080';

const state = { token: null, userId: null, role: null, entityId: null };

// ── Bootstrap ──────────────────────────────────────────────────────────────────

window.addEventListener('hashchange', router);
window.addEventListener('load', () => {
  const saved = localStorage.getItem('pc_token');
  if (saved) {
    try {
      const payload = JSON.parse(atob(saved.split('.')[1].replace(/-/g,'+').replace(/_/g,'/')));
      if (payload.exp > Date.now() / 1000) {
        state.token    = saved;
        state.userId   = payload.sub;
        state.role     = payload.role;
        state.entityId = payload.eid;
      }
    } catch (_) {}
  }
  router();
});

function router() {
  const hash = window.location.hash;
  if (hash.startsWith('#invite/')) {
    const tok = hash.slice('#invite/'.length);
    showInviteRegister(tok);
    return;
  }
  if (hash === '#bootstrap') { showSection('bootstrap'); hideChrome(); return; }
  if (!state.token) { showSection('login'); hideChrome(); return; }
  if (state.role === 'owner') { showSection('owner-dashboard'); showChrome(); loadOwnerDashboard(); return; }
  if (state.role === 'vet')   { showSection('vet-dashboard');   showChrome(); loadVetDashboard();   return; }
  if (state.role === 'admin') { showSection('admin-dashboard'); showChrome(); loadAdminDashboard(); return; }
  logout();
}

function showSection(id) {
  document.querySelectorAll('section').forEach(s => s.classList.remove('active'));
  const el = document.getElementById(id);
  if (el) el.classList.add('active');
}

function showChrome() {
  document.getElementById('header').classList.add('visible');
  document.getElementById('nav').classList.add('visible');
  document.getElementById('role-badge').textContent = state.role;
  const nav = document.getElementById('nav');
  nav.innerHTML = '';
  if (state.role === 'owner') {
    nav.innerHTML = `
      <button onclick="ownerNav('profile')">Profile</button>
      <button onclick="ownerNav('pets')">My Pets</button>
      <button onclick="ownerNav('vets')">Find a Vet</button>
      <span class="spacer"></span>
      <button class="logout-btn" onclick="logout()">Sign Out</button>`;
  } else if (state.role === 'vet') {
    nav.innerHTML = `
      <button onclick="vetNav('patients')">Patients</button>
      <button onclick="vetNav('upload')">Upload Doc</button>
      <button onclick="vetNav('viewdocs')">View Docs</button>
      <span class="spacer"></span>
      <button class="logout-btn" onclick="logout()">Sign Out</button>`;
  } else if (state.role === 'admin') {
    nav.innerHTML = `
      <button onclick="adminNav('invites')">Invites</button>
      <button onclick="adminNav('vets')">Vets</button>
      <span class="spacer"></span>
      <button class="logout-btn" onclick="logout()">Sign Out</button>`;
  }
}

function hideChrome() {
  document.getElementById('header').classList.remove('visible');
  document.getElementById('nav').classList.remove('visible');
}

function logout() {
  state.token = state.userId = state.role = state.entityId = null;
  localStorage.removeItem('pc_token');
  hideChrome();
  window.location.hash = '';
  showSection('login');
}

function saveSession(data) {
  state.token    = data.token;
  state.userId   = data.userId;
  state.role     = data.role;
  state.entityId = data.entityId || data.ownerId || data.vetId || null;
  localStorage.setItem('pc_token', data.token);
}

// ── Auth helpers ───────────────────────────────────────────────────────────────

async function authedFetch(url, opts = {}) {
  opts.headers = {
    'Content-Type': 'application/json',
    'Authorization': `Bearer ${state.token}`,
    ...(opts.headers || {}),
  };
  const r = await fetch(url, opts);
  if (r.status === 401) { logout(); return null; }
  return r;
}

async function post(url, body) {
  return fetch(url, { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
}

function showMsg(id, text, ok = true) {
  const el = document.getElementById(id);
  if (!el) return;
  el.textContent = text;
  el.className = 'auth-msg ' + (ok ? 'ok' : 'err');
}

function clearMsg(id) {
  const el = document.getElementById(id);
  if (el) { el.textContent = ''; el.className = 'auth-msg'; }
}

function show(id) { showSection(id); }

// ── Login ──────────────────────────────────────────────────────────────────────

async function doLogin(e) {
  e.preventDefault();
  clearMsg('login-msg');
  const r = await post(`${API}/auth/login`, {
    username: document.getElementById('l-user').value,
    password: document.getElementById('l-pass').value,
  });
  const data = await r.json();
  if (!r.ok) { showMsg('login-msg', data.error || 'Login failed', false); return; }
  saveSession(data);
  router();
}

// ── Register Owner ─────────────────────────────────────────────────────────────

async function doRegisterOwner(e) {
  e.preventDefault();
  clearMsg('reg-owner-msg');
  const r = await post(`${API}/auth/register-owner`, {
    username:  document.getElementById('ro-user').value,
    password:  document.getElementById('ro-pass').value,
    firstName: document.getElementById('ro-fn').value,
    lastName:  document.getElementById('ro-ln').value,
    address:   document.getElementById('ro-addr').value,
    city:      document.getElementById('ro-city').value,
    telephone: document.getElementById('ro-tel').value,
  });
  const data = await r.json();
  if (!r.ok) { showMsg('reg-owner-msg', data.error || 'Registration failed', false); return; }
  saveSession(data);
  router();
}

// ── Register Vet (invite) ──────────────────────────────────────────────────────

let _inviteToken = '';

function showInviteRegister(tok) {
  _inviteToken = tok;
  hideChrome();
  showSection('invite-register');
  const el = document.getElementById('invite-token-display');
  if (el) el.textContent = 'Invite code: ' + tok;
}

async function doRegisterVet(e) {
  e.preventDefault();
  clearMsg('reg-vet-msg');
  const r = await post(`${API}/auth/register-vet`, {
    inviteToken: _inviteToken,
    username:    document.getElementById('rv-user').value,
    password:    document.getElementById('rv-pass').value,
    firstName:   document.getElementById('rv-fn').value,
    lastName:    document.getElementById('rv-ln').value,
  });
  const data = await r.json();
  if (!r.ok) {
    const msgs = { 404: 'Invite not found.', 409: data.error, 410: 'Invite link has expired.' };
    showMsg('reg-vet-msg', msgs[r.status] || data.error || 'Registration failed', false);
    return;
  }
  saveSession(data);
  window.location.hash = '';
  router();
}

// ── Admin Bootstrap ────────────────────────────────────────────────────────────

async function doBootstrap(e) {
  e.preventDefault();
  clearMsg('bootstrap-msg');
  const r = await post(`${API}/admin/bootstrap`, {
    username: document.getElementById('bs-user').value,
    password: document.getElementById('bs-pass').value,
  });
  const data = await r.json();
  if (!r.ok) {
    if (r.status === 409) showMsg('bootstrap-msg', 'Admin already configured. Please sign in.', false);
    else showMsg('bootstrap-msg', data.error || 'Setup failed', false);
    return;
  }
  saveSession(data);
  window.location.hash = '';
  router();
}

// ── Owner Dashboard ────────────────────────────────────────────────────────────

async function loadOwnerDashboard() {
  const r = await authedFetch(`${API}/owners/${state.entityId}`);
  if (!r) return;
  const owner = await r.json();
  const info = document.getElementById('owner-profile-info');
  if (info) {
    info.innerHTML = `<strong>${owner.firstName} ${owner.lastName}</strong><br>
      ${owner.address}, ${owner.city}<br>
      Tel: ${owner.telephone}`;
    document.getElementById('up-addr').value = owner.address || '';
    document.getElementById('up-city').value = owner.city || '';
    document.getElementById('up-tel').value  = owner.telephone || '';
  }
  const petList = document.getElementById('pet-list');
  if (petList && owner.pets) {
    petList.innerHTML = owner.pets.length
      ? owner.pets.map(p => `<li><strong>${p.name}</strong> (${p.petType.name}) — born ${p.birthDate}<br>
          <small style="color:#888">ID: ${p.petId}</small></li>`).join('')
      : '<li style="color:#888">No pets yet.</li>';
  }
  loadOwnerVets();
}

async function loadOwnerVets() {
  const r = await authedFetch(`${API}/vets`);
  if (!r) return;
  const vets = await r.json();
  const list = document.getElementById('owner-vet-list');
  if (list) {
    list.innerHTML = vets.length
      ? vets.map(v => `<li><strong>Dr. ${v.firstName} ${v.lastName}</strong> — ${v.specialtyCount} specialt${v.specialtyCount !== 1 ? 'ies' : 'y'}</li>`).join('')
      : '<li style="color:#888">No vets registered.</li>';
  }
}

function ownerTab(tab, btn) {
  document.querySelectorAll('#owner-dashboard .tab').forEach(b => b.classList.remove('active'));
  document.querySelectorAll('#owner-dashboard .subsection').forEach(s => s.classList.remove('active'));
  btn.classList.add('active');
  const el = document.getElementById('owner-' + tab);
  if (el) el.classList.add('active');
  if (tab === 'vets') loadOwnerVets();
}

function ownerNav(tab) {
  showSection('owner-dashboard');
  const btn = document.querySelector(`#owner-dashboard .tab[onclick*="${tab}"]`);
  if (btn) ownerTab(tab, btn);
}

async function doUpdateOwner(e) {
  e.preventDefault();
  const r = await authedFetch(`${API}/owners/${state.entityId}`, {
    method: 'PUT',
    body: JSON.stringify({
      address:   document.getElementById('up-addr').value || undefined,
      city:      document.getElementById('up-city').value || undefined,
      telephone: document.getElementById('up-tel').value  || undefined,
    }),
  });
  if (r && r.ok) loadOwnerDashboard();
}

async function doAddPet(e) {
  e.preventDefault();
  const r = await authedFetch(`${API}/owners/${state.entityId}/pets`, {
    method: 'POST',
    body: JSON.stringify({
      name:        document.getElementById('ap-name').value,
      birthDate:   document.getElementById('ap-bd').value,
      petTypeId:   document.getElementById('ap-tid').value,
      petTypeName: document.getElementById('ap-tn').value,
    }),
  });
  if (r && r.ok) { loadOwnerDashboard(); e.target.reset(); }
}

// ── Vet Dashboard ──────────────────────────────────────────────────────────────

async function loadVetDashboard() {
  const r = await authedFetch(`${API}/owners`);
  if (!r) return;
  const owners = await r.json();
  const list = document.getElementById('vet-owner-list');
  if (list) {
    list.innerHTML = owners.length
      ? owners.map(o => `<li><strong>${o.firstName} ${o.lastName}</strong> — ${o.city} — ${o.petCount} pet${o.petCount !== 1 ? 's' : ''}</li>`).join('')
      : '<li style="color:#888">No owners registered.</li>';
  }
}

function vetTab(tab, btn) {
  document.querySelectorAll('#vet-dashboard .tab').forEach(b => b.classList.remove('active'));
  document.querySelectorAll('#vet-dashboard .subsection').forEach(s => s.classList.remove('active'));
  btn.classList.add('active');
  const el = document.getElementById('vet-' + tab);
  if (el) el.classList.add('active');
  if (tab === 'patients') loadVetDashboard();
}

function vetNav(tab) {
  showSection('vet-dashboard');
  const btn = document.querySelector(`#vet-dashboard .tab[onclick*="${tab}"]`);
  if (btn) vetTab(tab, btn);
}

async function doUploadDoc(e) {
  e.preventDefault();
  clearMsg('upload-msg');
  const petId = document.getElementById('ud-pet').value;
  const fileInput = document.getElementById('ud-file');
  let attachments = [];
  if (fileInput.files.length > 0) {
    const file = fileInput.files[0];
    const dataBase64 = await new Promise(res => {
      const reader = new FileReader();
      reader.onload = ev => res(ev.target.result.split(',')[1]);
      reader.readAsDataURL(file);
    });
    attachments = [{ filename: file.name, contentType: file.type || 'application/octet-stream', dataBase64 }];
  }
  const r = await authedFetch(`${API}/pets/${petId}/medical-documents`, {
    method: 'POST',
    body: JSON.stringify({
      title:       document.getElementById('ud-title').value,
      notes:       document.getElementById('ud-notes').value,
      attachments,
    }),
  });
  if (!r) return;
  const data = await r.json();
  if (r.ok) { showMsg('upload-msg', `Document uploaded (ID: ${data.docId})`, true); e.target.reset(); }
  else showMsg('upload-msg', data.error || 'Upload failed', false);
}

async function doLoadDocs(e) {
  e.preventDefault();
  const petId = document.getElementById('vd-pet').value;
  const r = await authedFetch(`${API}/pets/${petId}/medical-documents`);
  if (!r) return;
  const docs = await r.json();
  const list = document.getElementById('docs-list');
  if (!list) return;
  if (!docs.length) { list.innerHTML = '<p style="color:#888;padding:1rem">No documents found.</p>'; return; }
  list.innerHTML = docs.map(d => `
    <div class="card" style="margin-bottom:0.75rem">
      <strong>${d.title}</strong> <span style="color:#888;font-size:0.85rem">${new Date(d.createdAt*1000).toLocaleDateString()}</span><br>
      <p style="margin:0.5rem 0;color:#555">${d.notes}</p>
      <small style="color:#aaa">${d.attachmentCount} attachment${d.attachmentCount !== 1 ? 's' : ''} — Vet: ${d.vetId.slice(0,8)}…</small>
    </div>`).join('');
}

// ── Admin Dashboard ────────────────────────────────────────────────────────────

async function loadAdminDashboard() {
  loadInvites();
  loadAdminVets();
}

async function loadInvites() {
  const r = await authedFetch(`${API}/admin/invites`);
  if (!r) return;
  const invites = await r.json();
  const tbody = document.getElementById('invites-body');
  if (!tbody) return;
  const now = Date.now() / 1000;
  tbody.innerHTML = invites.length
    ? invites.map(inv => {
        let badge;
        if (inv.used) badge = '<span class="badge used">Used</span>';
        else if (inv.expiresAt < now) badge = '<span class="badge expired">Expired</span>';
        else badge = '<span class="badge active">Active</span>';
        return `<tr>
          <td><code style="font-size:0.8rem">${inv.token.slice(0,8)}…</code></td>
          <td>${new Date(inv.expiresAt*1000).toLocaleDateString()}</td>
          <td>${badge}</td>
        </tr>`;
      }).join('')
    : '<tr><td colspan="3" style="color:#aaa;text-align:center;padding:1.5rem">No invites yet.</td></tr>';
}

async function loadAdminVets() {
  const r = await authedFetch(`${API}/vets`);
  if (!r) return;
  const vets = await r.json();
  const list = document.getElementById('admin-vet-list');
  if (list) {
    list.innerHTML = vets.length
      ? vets.map(v => `<li><strong>Dr. ${v.firstName} ${v.lastName}</strong> — ${v.specialtyCount} specialt${v.specialtyCount !== 1 ? 'ies' : 'y'}</li>`).join('')
      : '<li style="color:#888">No vets registered.</li>';
  }
}

async function doCreateInvite() {
  const r = await authedFetch(`${API}/admin/invites`, { method: 'POST', body: '{}' });
  if (!r) return;
  const data = await r.json();
  if (!r.ok) { showMsg('admin-msg', data.error || 'Failed', false); return; }
  clearMsg('admin-msg');
  const link = `${window.location.origin}${data.link}`;
  const box = document.getElementById('new-invite-result');
  if (box) {
    box.innerHTML = `
      <div style="margin-top:0.75rem;font-size:0.85rem;color:#555">Share this link with the doctor:</div>
      <div class="invite-link-box">
        <input type="text" value="${link}" readonly id="invite-link-val" />
        <button class="copy-btn" onclick="copyInvite()">Copy</button>
      </div>
      <div style="font-size:0.8rem;color:#888">Expires: ${new Date(data.expiresAt*1000).toLocaleString()}</div>`;
  }
  loadInvites();
}

function copyInvite() {
  const el = document.getElementById('invite-link-val');
  if (el) { el.select(); document.execCommand('copy'); }
}

function adminTab(tab, btn) {
  document.querySelectorAll('#admin-dashboard .tab').forEach(b => b.classList.remove('active'));
  document.querySelectorAll('#admin-dashboard .subsection').forEach(s => s.classList.remove('active'));
  btn.classList.add('active');
  const el = document.getElementById('admin-' + tab);
  if (el) el.classList.add('active');
  if (tab === 'invites') loadInvites();
  if (tab === 'vets') loadAdminVets();
}

function adminNav(tab) {
  showSection('admin-dashboard');
  const btn = document.querySelector(`#admin-dashboard .tab[onclick*="${tab}"]`);
  if (btn) adminTab(tab, btn);
}

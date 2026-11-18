// REDSHIFT account client: Supabase Auth + PostgREST (profiles) + Storage (skins).
// Talks to the same Supabase project as the website, using only the publishable key and the
// signed-in user's own JWT, so Row Level Security stays in charge. No secrets, no service_role.
//
// It uses the official Supabase HTTP APIs directly (GoTrue /auth/v1, PostgREST /rest/v1, Storage /storage/v1).
const { validateSkin } = require('./pngcheck');

const SESSION_KEY = 'redshift.session';

class RsError extends Error {
  constructor(code, message) { super(message); this.code = code; }
}

const AUTH_MESSAGES = {
  invalid_credentials: 'Неверный email или пароль',
  email_not_confirmed: 'Email не подтверждён. Откройте письмо от REDSHIFT и перейдите по ссылке, затем войдите снова',
  user_banned: 'Этот аккаунт заблокирован',
  over_request_rate_limit: 'Слишком много попыток входа. Подождите немного',
  over_email_send_rate_limit: 'Слишком много попыток. Подождите немного',
  user_not_found: 'Аккаунт с таким email не найден',
  validation_failed: 'Проверьте email и пароль'
};

function create({ getConfig, store, fetchImpl = fetch, now = () => Date.now() }) {
  let session = null;      // { access_token, refresh_token, expires_at (ms), user: {id, email} }
  let refreshing = null;   // single in-flight refresh (refresh tokens rotate, parallel refreshes would break the session)
  let profileCache = null;

  const cfg = () => getConfig();
  const need = () => {
    const c = cfg();
    if (c.keyProblem) throw new RsError('config', c.keyProblem);
    if (!c.configured) throw new RsError('config', 'Не задан Publishable Key Supabase. Откройте Настройки → REDSHIFT и вставьте ключ с сайта (Project Settings → API Keys)');
    return c;
  };

  async function http(url, opts = {}) {
    try {
      return await fetchImpl(url, { ...opts, signal: AbortSignal.timeout(20000) });
    } catch (e) {
      throw new RsError('network', 'Нет соединения с сервером REDSHIFT. Проверьте интернет и попробуйте ещё раз');
    }
  }

  async function readBody(r) {
    const text = await r.text();
    try { return text ? JSON.parse(text) : {}; } catch { return { message: text }; }
  }

  function authError(r, body) {
    const code = body.error_code || body.code || body.error || '';
    if (AUTH_MESSAGES[code]) return new RsError(code, AUTH_MESSAGES[code]);
    const text = String(body.msg || body.error_description || body.message || '');
    if (/email not confirmed/i.test(text)) return new RsError('email_not_confirmed', AUTH_MESSAGES.email_not_confirmed);
    if (/invalid login credentials/i.test(text)) return new RsError('invalid_credentials', AUTH_MESSAGES.invalid_credentials);
    if (r.status === 429) return new RsError('over_request_rate_limit', AUTH_MESSAGES.over_request_rate_limit);
    if (r.status === 401 || r.status === 403) return new RsError('forbidden', 'Сервер отклонил запрос. Проверьте Publishable Key в настройках');
    if (r.status >= 500) return new RsError('server', 'Сервер REDSHIFT временно недоступен. Попробуйте позже');
    return new RsError('auth', text || `Ошибка входа (${r.status})`);
  }

  function setSession(body) {
    const expiresIn = Number(body.expires_in) || 3600;
    session = {
      access_token: body.access_token,
      refresh_token: body.refresh_token,
      expires_at: now() + expiresIn * 1000,
      user: { id: body.user && body.user.id, email: body.user && body.user.email }
    };
    if (!session.user.id && session.access_token) {          // fall back to the JWT "sub"
      try { session.user.id = JSON.parse(Buffer.from(session.access_token.split('.')[1], 'base64url').toString()).sub; } catch {}
    }
    store.set(SESSION_KEY, session);
  }

  function clearSession() { session = null; profileCache = null; store.remove(SESSION_KEY); }

  const authHeaders = (c, token) => ({ apikey: c.supabasePublishableKey, Authorization: `Bearer ${token || c.supabasePublishableKey}` });

  // ----- sign in / out -----
  async function signIn(email, password) {
    const c = need();
    email = String(email || '').trim().toLowerCase();
    if (!email || !password) throw new RsError('validation_failed', 'Введите email и пароль');
    const r = await http(`${c.supabaseUrl}/auth/v1/token?grant_type=password`, {
      method: 'POST',
      headers: { ...authHeaders(c), 'Content-Type': 'application/json' },
      body: JSON.stringify({ email, password })
    });
    const body = await readBody(r);
    if (!r.ok) throw authError(r, body);
    setSession(body);
    profileCache = null;
    return status();
  }

  async function signOut() {
    const c = cfg();
    const token = session && session.access_token;
    clearSession();                               // local sign-out always succeeds, whatever the network does
    if (token && c.configured) {
      try {
        await http(`${c.supabaseUrl}/auth/v1/logout?scope=local`, { method: 'POST', headers: authHeaders(c, token) });
      } catch { /* offline: the local session is already gone */ }
    }
    return status();
  }

  // ----- session lifecycle -----
  async function doRefresh() {
    const c = need();
    const r = await http(`${c.supabaseUrl}/auth/v1/token?grant_type=refresh_token`, {
      method: 'POST',
      headers: { ...authHeaders(c), 'Content-Type': 'application/json' },
      body: JSON.stringify({ refresh_token: session.refresh_token })
    });
    const body = await readBody(r);
    if (!r.ok) {
      if (r.status >= 400 && r.status < 500) {      // refresh token revoked / already used / expired
        clearSession();
        throw new RsError('session_expired', 'Сессия истекла. Войдите в аккаунт REDSHIFT заново');
      }
      throw authError(r, body);
    }
    const prevUser = session.user;
    setSession({ ...body, user: body.user || prevUser });
  }

  function refresh() {
    if (!refreshing) refreshing = doRefresh().finally(() => { refreshing = null; });
    return refreshing;
  }

  async function token() {
    if (!session) throw new RsError('signed_out', 'Войдите в аккаунт REDSHIFT');
    if (session.expires_at - 60000 <= now()) await refresh();
    return session.access_token;
  }

  // Load the saved session on app start. Never throws: returns the resulting status.
  async function restore() {
    const saved = store.get(SESSION_KEY);
    if (!saved || !saved.refresh_token) return status();
    session = saved;
    if (!cfg().configured) return status();
    try {
      if (session.expires_at - 60000 <= now()) await refresh();
    } catch (e) {
      if (e.code === 'network') return { ...status(), offline: true };   // keep the session, try again later
      if (e.code !== 'session_expired') { return { ...status(), error: e.message }; }
    }
    return status();
  }

  function status() {
    const c = cfg();
    return {
      configured: !!c.configured,
      configProblem: c.keyProblem || '',
      signedIn: !!session,
      user: session ? { id: session.user.id, email: session.user.email } : null,
      persistent: store.persistent(),
      siteUrl: c.siteUrl || ''
    };
  }

  // Authenticated request with one automatic retry after a 401 (expired JWT).
  async function authed(path, opts = {}, retry = true) {
    const c = need();
    const t = await token();
    const r = await http(`${c.supabaseUrl}${path}`, { ...opts, headers: { ...authHeaders(c, t), ...(opts.headers || {}) } });
    if (r.status === 401 && retry) { await refresh(); return authed(path, opts, false); }
    return r;
  }

  function dataError(r, body, what) {
    const text = String(body.message || body.error || body.msg || '');
    if (r.status === 401) return new RsError('session_expired', 'Сессия истекла. Войдите заново');
    if (r.status === 403 || body.code === '42501' || /row-level security|violates/i.test(text)) {
      return new RsError('forbidden', `Нет прав на «${what}». Проверьте RLS-политики на сайте: пользователь должен писать только в свою папку (<user id>/…)`);
    }
    if (r.status === 404) return new RsError('not_found', `${what}: не найдено`);
    if (r.status === 413) return new RsError('too_large', 'Файл слишком большой для хранилища');
    if (r.status >= 500) return new RsError('server', 'Сервер REDSHIFT временно недоступен');
    return new RsError('data', `${what}: ${text || r.status}`);
  }

  // ----- profile -----
  async function getProfile(force = false) {
    if (profileCache && !force) return profileCache;
    const c = need();
    await token();
    const uid = session.user.id;
    const r = await authed(`/rest/v1/${c.profilesTable}?id=eq.${encodeURIComponent(uid)}&select=id,username,skin_path,updated_at`);
    const body = await readBody(r);
    if (!r.ok) throw dataError(r, body, 'Профиль');
    const row = Array.isArray(body) ? body[0] : null;
    profileCache = row || { id: uid, username: null, skin_path: null, updated_at: null, missing: true };
    return profileCache;
  }

  // ----- skin -----
  const encPath = (p) => p.split('/').map(encodeURIComponent).join('/');

  async function downloadSkin(skinPath) {
    const c = need();
    if (!skinPath) return null;
    // bucket may be private (needs the user's JWT) or public
    let r = await authed(`/storage/v1/object/authenticated/${c.skinsBucket}/${encPath(skinPath)}`);
    if (r.status === 404 || r.status === 400) {
      r = await http(`${c.supabaseUrl}/storage/v1/object/public/${c.skinsBucket}/${encPath(skinPath)}`, { headers: { apikey: c.supabasePublishableKey } });
    }
    if (r.status === 404 || r.status === 400) return null;       // no such file: show a default skin
    if (!r.ok) throw dataError(r, await readBody(r), 'Скин');
    const buf = Buffer.from(await r.arrayBuffer());
    const v = validateSkin(buf, { sizes: c.skinSizes, maxBytes: c.skinMaxBytes * 4 });
    if (!v.ok) throw new RsError('bad_skin', `Скин на сервере повреждён: ${v.reason}`);
    return buf;
  }

  async function getSkin() {
    const profile = await getProfile();
    const buf = await downloadSkin(profile.skin_path);
    return { buffer: buf, path: profile.skin_path || null };
  }

  async function uploadSkin(buf) {
    const c = need();
    const v = validateSkin(buf, { sizes: c.skinSizes, maxBytes: c.skinMaxBytes });
    if (!v.ok) throw new RsError('bad_skin', v.reason);
    const profile = await getProfile(true);
    if (profile.missing) throw new RsError('no_profile', 'Профиль не найден. Сначала завершите регистрацию на сайте REDSHIFT');
    const uid = session.user.id;
    const skinPath = profile.skin_path || c.skinPathTemplate.replace('{uid}', uid);
    if (!skinPath.startsWith(uid + '/') && !skinPath.startsWith(uid + '.')) {
      // refuse to write outside the user's own folder, whatever the profile row says
      throw new RsError('forbidden', 'Путь скина в профиле не принадлежит вашему аккаунту — загрузка отменена');
    }
    let r = await authed(`/storage/v1/object/${c.skinsBucket}/${encPath(skinPath)}`, {
      method: 'POST',
      headers: { 'Content-Type': 'image/png', 'x-upsert': 'true', 'cache-control': '3600' },
      body: buf
    });
    if (!r.ok) throw dataError(r, await readBody(r), 'Загрузка скина');

    const patch = async (withDate) => authed(`/rest/v1/${c.profilesTable}?id=eq.${encodeURIComponent(uid)}`, {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json', Prefer: 'return=representation' },
      body: JSON.stringify(withDate ? { skin_path: skinPath, updated_at: new Date(now()).toISOString() } : { skin_path: skinPath })
    });
    r = await patch(true);
    let body = await readBody(r);
    if (!r.ok && (body.code === 'PGRST204' || /updated_at/.test(String(body.message)))) {   // schema has no updated_at
      r = await patch(false); body = await readBody(r);
    }
    if (!r.ok) throw dataError(r, body, 'Сохранение профиля');
    if (Array.isArray(body) && body.length === 0) throw new RsError('forbidden', 'Профиль не обновлён (проверьте RLS-политику UPDATE на таблице profiles)');
    profileCache = Array.isArray(body) && body[0] ? { ...profileCache, ...body[0] } : { ...profileCache, skin_path: skinPath };
    return { path: skinPath, profile: profileCache };
  }

  return { signIn, signOut, restore, status, getProfile, getSkin, uploadSkin, token, RsError };
}

module.exports = { create, RsError };

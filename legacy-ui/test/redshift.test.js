const test = require('node:test');
const assert = require('node:assert/strict');
const fs = require('fs');
const os = require('os');
const path = require('path');
const { startSupabase, makePng, fakeSafeStorage, jwt } = require('./helpers');
const config = require('../src/config');
const { createStore } = require('../src/secure');
const redshift = require('../src/redshift');

const ALICE = '11111111-1111-1111-1111-111111111111';
const KEY = 'sb_publishable_test_key';

function setup(t, srv, { clock } = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'rs-'));
  const file = path.join(dir, 'session.bin');
  const cfg = { ...config.DEFAULTS, supabaseUrl: srv.url, supabasePublishableKey: KEY, configured: true, keyProblem: '' };
  const make = () => redshift.create({
    getConfig: () => cfg, store: createStore({ file, safeStorage: fakeSafeStorage }), now: clock || Date.now
  });
  t.after(() => fs.rmSync(dir, { recursive: true, force: true }));
  return { make, file, cfg, dir };
}

test('sign in: valid credentials give a session for the same user id as the website', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const { make } = setup(t, srv);
  const rs = make();
  const st = await rs.signIn('  Alice@Example.com ', 'correct-horse');
  assert.equal(st.signedIn, true);
  assert.equal(st.user.id, ALICE);
  assert.equal(st.user.email, 'alice@example.com');
});

test('sign in: wrong password -> friendly message, no session', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const rs = setup(t, srv).make();
  await assert.rejects(() => rs.signIn('alice@example.com', 'nope'), (e) => e.code === 'invalid_credentials' && /Неверный email или пароль/.test(e.message));
  assert.equal(rs.status().signedIn, false);
});

test('sign in: unconfirmed email is explained', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const rs = setup(t, srv).make();
  await assert.rejects(() => rs.signIn('bob@example.com', 'hunter22'), (e) => e.code === 'email_not_confirmed' && /не подтверждён/.test(e.message));
});

test('sign in: empty fields are rejected without a network call', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const rs = setup(t, srv).make();
  await assert.rejects(() => rs.signIn('', ''), /Введите email и пароль/);
  assert.equal(srv.calls.password, 0);
});

test('network failure gives a readable error', async (t) => {
  const srv = await startSupabase();
  const rs = setup(t, srv).make();
  await srv.close();
  await assert.rejects(() => rs.signIn('alice@example.com', 'correct-horse'), (e) => e.code === 'network' && /Нет соединения/.test(e.message));
});

test('session is stored encrypted: no token or password in the file on disk', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const { make, file } = setup(t, srv);
  await make().signIn('alice@example.com', 'correct-horse');
  const raw = fs.readFileSync(file).toString('latin1');
  assert.ok(raw.length > 0);
  assert.ok(!raw.includes('at_'), 'access token must not be readable');
  assert.ok(!raw.includes('rt_'), 'refresh token must not be readable');
  assert.ok(!raw.includes('correct-horse'), 'password must never be stored');
});

test('restore: a new app run is still signed in without a password', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const { make } = setup(t, srv);
  await make().signIn('alice@example.com', 'correct-horse');
  const st = await make().restore();                       // simulates restart
  assert.equal(st.signedIn, true);
  assert.equal(st.user.id, ALICE);
  assert.equal(srv.calls.password, 1, 'must not sign in with a password again');
});

test('restore: expired access token is refreshed and the rotated token is saved', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  let time = Date.now();
  const { make } = setup(t, srv, { clock: () => time });
  await make().signIn('alice@example.com', 'correct-horse');
  time += 2 * 3600 * 1000;                                 // two hours later
  const st = await make().restore();
  assert.equal(st.signedIn, true);
  assert.equal(srv.calls.refresh, 1);
  const again = await make().restore();                    // the rotated token must work after another restart
  assert.equal(again.signedIn, true);
});

test('restore: revoked refresh token signs the user out and clears storage', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  let time = Date.now();
  const { make, file } = setup(t, srv, { clock: () => time });
  await make().signIn('alice@example.com', 'correct-horse');
  srv.refresh.forEach((r) => { r.used = true; });          // server revoked it
  time += 2 * 3600 * 1000;
  const st = await make().restore();
  assert.equal(st.signedIn, false);
  const rs2 = make();
  assert.equal((await rs2.restore()).signedIn, false, 'stored session must be gone');
});

test('restore: server unreachable keeps the session (offline), no forced logout', async (t) => {
  const srv = await startSupabase();
  let time = Date.now();
  const { make } = setup(t, srv, { clock: () => time });
  await make().signIn('alice@example.com', 'correct-horse');
  time += 2 * 3600 * 1000;
  srv.state.down = true;
  const st = await make().restore();
  assert.equal(st.signedIn, true);
  assert.equal(st.offline, true);
  srv.state.down = false;
  await srv.close();
});

test('sign out: clears local session and calls the server; works offline too', async (t) => {
  const srv = await startSupabase();
  const { make } = setup(t, srv);
  const rs = make();
  await rs.signIn('alice@example.com', 'correct-horse');
  const st = await rs.signOut();
  assert.equal(st.signedIn, false);
  assert.equal(srv.calls.logout, 1);
  assert.equal((await make().restore()).signedIn, false, 'nothing left in storage');

  const rs2 = make();
  await rs2.signIn('alice@example.com', 'correct-horse');
  srv.state.down = true;
  assert.equal((await rs2.signOut()).signedIn, false, 'offline sign-out must still succeed');
  await srv.close();
});

test('profile: username and skin_path come from the shared profiles table', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const rs = setup(t, srv).make();
  await rs.signIn('alice@example.com', 'correct-horse');
  const p = await rs.getProfile();
  assert.equal(p.id, ALICE);
  assert.equal(p.username, 'AliceMC');
});

test('401 during a request: one refresh, then the request is retried', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const rs = setup(t, srv).make();
  await rs.signIn('alice@example.com', 'correct-horse');
  srv.expireAllAccessTokens();
  const p = await rs.getProfile(true);
  assert.equal(p.username, 'AliceMC');
  assert.equal(srv.calls.refresh, 1);
});

test('parallel requests with an expired token cause exactly one refresh', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  let time = Date.now();
  const rs = setup(t, srv, { clock: () => time }).make();
  await rs.signIn('alice@example.com', 'correct-horse');
  time += 2 * 3600 * 1000;
  await Promise.all([rs.getProfile(true), rs.getProfile(true), rs.getProfile(true)]);
  assert.equal(srv.calls.refresh, 1);
});

test('skin: invalid files are rejected locally before any upload', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const rs = setup(t, srv).make();
  await rs.signIn('alice@example.com', 'correct-horse');
  await assert.rejects(() => rs.uploadSkin(Buffer.from('definitely not a png.png')), (e) => e.code === 'bad_skin');
  await assert.rejects(() => rs.uploadSkin(makePng(128, 128)), /64×64 или 64×32/);
  await assert.rejects(() => rs.uploadSkin(Buffer.concat([makePng(64, 64).subarray(0, 40)])), (e) => e.code === 'bad_skin');
  assert.equal(srv.calls.upload, 0);
});

test('skin: upload goes to <user id>/skin.png, profile.skin_path is updated and the file reads back', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const rs = setup(t, srv).make();
  await rs.signIn('alice@example.com', 'correct-horse');
  const png = makePng(64, 64);
  const r = await rs.uploadSkin(png);
  assert.equal(r.path, `${ALICE}/skin.png`);
  assert.equal(srv.profiles[ALICE].skin_path, `${ALICE}/skin.png`);
  assert.ok(srv.profiles[ALICE].updated_at);
  assert.ok(srv.files.get(`skins/${ALICE}/skin.png`).equals(png));
  const got = await rs.getSkin();
  assert.ok(got.buffer.equals(png));
  const legacy = await rs.uploadSkin(makePng(64, 32));     // legacy 64x32 is accepted as well
  assert.equal(legacy.path, `${ALICE}/skin.png`);
});

test('skin: missing file on the server is not an error', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const rs = setup(t, srv).make();
  await rs.signIn('alice@example.com', 'correct-horse');
  srv.profiles[ALICE].skin_path = `${ALICE}/gone.png`;
  const got = await rs.getSkin();
  assert.equal(got.buffer, null);
});

test('skin: a profile pointing into someone else\'s folder is never written to', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const rs = setup(t, srv).make();
  await rs.signIn('alice@example.com', 'correct-horse');
  srv.profiles[ALICE].skin_path = '22222222-2222-2222-2222-222222222222/skin.png';
  await assert.rejects(() => rs.uploadSkin(makePng(64, 64)), (e) => e.code === 'forbidden');
  assert.equal(srv.calls.upload, 0);
});

test('skin: schema without updated_at is handled (retry without it)', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  srv.state.profileColumns = ['id', 'username', 'skin_path'];
  const rs = setup(t, srv).make();
  await rs.signIn('alice@example.com', 'correct-horse');
  const r = await rs.uploadSkin(makePng(64, 64));
  assert.equal(srv.profiles[ALICE].skin_path, r.path);
});

test('skin: server-side RLS denial is reported clearly', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const { make, cfg } = setup(t, srv);
  const rs = make();
  await rs.signIn('alice@example.com', 'correct-horse');
  // passes the client-side own-folder guard, but the (mock) RLS policy only allows "<uid>/…" folders
  cfg.skinPathTemplate = '{uid}.png';
  srv.profiles[ALICE].skin_path = null;
  await assert.rejects(() => rs.uploadSkin(makePng(64, 64)), (e) => e.code === 'forbidden' && /RLS/.test(e.message));
  assert.equal(srv.calls.upload, 1, 'the request did reach the server and was denied there');
  assert.equal(srv.profiles[ALICE].skin_path, null, 'profile must not point to a file that was not stored');
});

test('skin: client-side guard blocks paths outside the user folder before any request', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const { make, cfg } = setup(t, srv);
  const rs = make();
  await rs.signIn('alice@example.com', 'correct-horse');
  cfg.skinPathTemplate = 'shared/{uid}.png';
  srv.profiles[ALICE].skin_path = null;
  await assert.rejects(() => rs.uploadSkin(makePng(64, 64)), (e) => e.code === 'forbidden');
  assert.equal(srv.calls.upload, 0);
});

test('config: default project is the website project; privileged keys are refused', () => {
  const c = config.load({ env: {} });
  assert.equal(c.supabaseUrl, 'https://hkyuclomrgnochqslqlp.supabase.co');
  assert.equal(c.configured, false, 'no key shipped in the source');

  const serviceRole = config.load({ env: {}, overrides: { supabaseKey: jwt({ role: 'service_role' }) } });
  assert.equal(serviceRole.configured, false);
  assert.match(serviceRole.keyProblem, /service_role/);

  const secret = config.load({ env: {}, overrides: { supabaseKey: 'sb_secret_abc' } });
  assert.equal(secret.configured, false);

  const anon = config.load({ env: {}, overrides: { supabaseKey: jwt({ role: 'anon' }) } });
  assert.equal(anon.configured, true);
  const pub = config.load({ env: { REDSHIFT_SUPABASE_KEY: 'sb_publishable_x' } });
  assert.equal(pub.configured, true);
});

test('not configured: sign-in explains what to do instead of failing obscurely', async (t) => {
  const srv = await startSupabase(); t.after(() => srv.close());
  const { make, cfg } = setup(t, srv);
  cfg.configured = false; cfg.supabasePublishableKey = '';
  await assert.rejects(() => make().signIn('a@b.c', 'x'), (e) => e.code === 'config' && /Publishable Key/.test(e.message));
});

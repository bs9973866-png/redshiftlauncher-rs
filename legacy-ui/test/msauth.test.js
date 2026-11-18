const test = require('node:test');
const assert = require('node:assert/strict');
const http = require('http');
const msauth = require('../src/msauth');

async function startMock(opts = {}) {
  const calls = { device: 0, token: 0, xbl: 0, xsts: 0, mc: 0, profile: 0 };
  const pending = { n: opts.pendingPolls ?? 2 };
  const server = http.createServer(async (req, res) => {
    const chunks = []; for await (const c of req) chunks.push(c);
    const raw = Buffer.concat(chunks).toString();
    const send = (code, body) => { res.writeHead(code, { 'Content-Type': 'application/json' }); res.end(JSON.stringify(body)); };
    const u = req.url;
    if (u === '/devicecode') { calls.device++; return send(200, { device_code: 'DEV', user_code: 'ABCD-1234', verification_uri: 'https://microsoft.com/link', interval: 5, expires_in: 900 }); }
    if (u === '/token') {
      calls.token++;
      const f = new URLSearchParams(raw);
      if (f.get('grant_type') === 'refresh_token') {
        return f.get('refresh_token') === 'MS_REFRESH_1' ? send(200, { access_token: 'MS_AT_2', refresh_token: 'MS_REFRESH_2' }) : send(400, { error: 'invalid_grant' });
      }
      if (opts.declined) return send(400, { error: 'authorization_declined' });
      if (pending.n-- > 0) return send(400, { error: pending.n === 0 && opts.slowDown ? 'slow_down' : 'authorization_pending' });
      return send(200, { access_token: 'MS_AT_1', refresh_token: 'MS_REFRESH_1' });
    }
    if (u === '/xbl') {
      calls.xbl++;
      const j = JSON.parse(raw);
      assert.equal(j.Properties.RpsTicket.startsWith('d='), true);
      return send(200, { Token: 'XBL_TOKEN' });
    }
    if (u === '/xsts') {
      calls.xsts++;
      if (opts.xerr) return send(401, { XErr: opts.xerr });
      const j = JSON.parse(raw);
      assert.deepEqual(j.Properties.UserTokens, ['XBL_TOKEN']);
      assert.equal(j.RelyingParty, 'rp://api.minecraftservices.com/');
      return send(200, { Token: 'XSTS_TOKEN', DisplayClaims: { xui: [{ uhs: 'UHS123' }] } });
    }
    if (u === '/mclogin') {
      calls.mc++;
      if (opts.mc403) return send(403, { error: 'Forbidden' });
      assert.equal(JSON.parse(raw).identityToken, 'XBL3.0 x=UHS123;XSTS_TOKEN');
      return send(200, { access_token: 'MC_TOKEN', expires_in: 86400 });
    }
    if (u === '/profile') {
      calls.profile++;
      assert.equal(req.headers.authorization, 'Bearer MC_TOKEN');
      if (opts.noGame) return send(404, { error: 'NOT_FOUND' });
      return send(200, { id: 'abcdef0123456789abcdef0123456789', name: 'Steve_Licensed', skins: [{ state: 'ACTIVE', variant: 'SLIM', url: 'https://textures.minecraft.net/texture/abc' }] });
    }
    send(404, {});
  });
  await new Promise((r) => server.listen(0, '127.0.0.1', r));
  const base = `http://127.0.0.1:${server.address().port}`;
  const urls = { deviceCode: base + '/devicecode', token: base + '/token', xblAuth: base + '/xbl', xstsAuth: base + '/xsts', mcLogin: base + '/mclogin', mcProfile: base + '/profile' };
  return { urls, calls, close: () => new Promise((r) => server.close(r)) };
}

const make = (urls, clientId = 'client-id-123') => msauth.create({ getClientId: () => clientId, urls, sleep: async () => {} });

test('device code flow: polls until the user confirms, then builds a Minecraft session', async (t) => {
  const m = await startMock({ pendingPolls: 2, slowDown: true }); t.after(m.close);
  const ms = make(m.urls);
  const dc = await ms.startDeviceLogin();
  assert.equal(dc.userCode, 'ABCD-1234');
  assert.equal(dc.verificationUri, 'https://microsoft.com/link');
  const tokens = await ms.waitForToken(dc);
  assert.equal(tokens.access_token, 'MS_AT_1');
  assert.equal(m.calls.token, 3, 'two pending/slow_down polls, then success');
  const s = await ms.sessionFromMicrosoft(tokens);
  assert.equal(s.name, 'Steve_Licensed');
  assert.equal(s.uuid, 'abcdef0123456789abcdef0123456789');
  assert.equal(s.mcAccessToken, 'MC_TOKEN');
  assert.equal(s.msRefreshToken, 'MS_REFRESH_1');
  assert.equal(s.skinModel, 'slim');
  assert.match(s.skinUrl, /textures\.minecraft\.net/);
});

test('refresh: new Minecraft token from a saved Microsoft refresh token, refresh token rotates', async (t) => {
  const m = await startMock(); t.after(m.close);
  const s = await make(m.urls).refreshSession('MS_REFRESH_1');
  assert.equal(s.mcAccessToken, 'MC_TOKEN');
  assert.equal(s.msRefreshToken, 'MS_REFRESH_2');
  await assert.rejects(() => make(m.urls).refreshSession('REVOKED'), (e) => e.code === 'session_expired');
});

test('declined in browser', async (t) => {
  const m = await startMock({ declined: true }); t.after(m.close);
  const ms = make(m.urls);
  await assert.rejects(async () => ms.waitForToken(await ms.startDeviceLogin()), (e) => e.code === 'declined');
});

test('user can cancel while waiting', async (t) => {
  const m = await startMock({ pendingPolls: 99 }); t.after(m.close);
  const ms = make(m.urls);
  const dc = await ms.startDeviceLogin();
  await assert.rejects(() => ms.waitForToken(dc, { cancelled: () => true }), (e) => e.code === 'cancelled');
});

test('Xbox account problems are explained in Russian', async (t) => {
  const m = await startMock({ xerr: 2148916233 }); t.after(m.close);
  await assert.rejects(() => make(m.urls).sessionFromMicrosoft({ access_token: 'x', refresh_token: 'y' }), (e) => e.code === 'xsts' && /нет профиля Xbox/.test(e.message));
  const kid = await startMock({ xerr: 2148916238 }); t.after(kid.close);
  await assert.rejects(() => make(kid.urls).sessionFromMicrosoft({ access_token: 'x', refresh_token: 'y' }), /родител/);
});

test('account without Minecraft Java is reported as no_game', async (t) => {
  const m = await startMock({ noGame: true }); t.after(m.close);
  await assert.rejects(() => make(m.urls).sessionFromMicrosoft({ access_token: 'x', refresh_token: 'y' }), (e) => e.code === 'no_game' && /нет лицензии/.test(e.message));
});

test('app not approved by Mojang (403) is explained', async (t) => {
  const m = await startMock({ mc403: true }); t.after(m.close);
  await assert.rejects(() => make(m.urls).sessionFromMicrosoft({ access_token: 'x', refresh_token: 'y' }), (e) => e.code === 'app_not_approved');
});

test('missing Client ID gives setup instructions, no request is made', async (t) => {
  const m = await startMock(); t.after(m.close);
  await assert.rejects(() => make(m.urls, '').startDeviceLogin(), (e) => e.code === 'config' && /Client ID/.test(e.message));
  assert.equal(m.calls.device, 0);
});

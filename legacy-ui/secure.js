// Encrypted key/value store for tokens. Uses the OS credential store through Electron's safeStorage
// (DPAPI on Windows, Keychain on macOS, libsecret/kwallet on Linux). If encryption is not available
// nothing is written to disk: the session then lives in memory only (user signs in again next launch).
const fs = require('fs');
const path = require('path');

function createStore({ file, safeStorage }) {
  const mem = new Map();
  const canEncrypt = () => {
    try { return !!(safeStorage && safeStorage.isEncryptionAvailable()); } catch { return false; }
  };

  function readAll() {
    if (!canEncrypt() || !fs.existsSync(file)) return {};
    try {
      const raw = fs.readFileSync(file);
      return JSON.parse(safeStorage.decryptString(raw));
    } catch { return {}; }          // corrupted / written by another OS user: treat as empty
  }
  function writeAll(obj) {
    if (!canEncrypt()) return false;
    fs.mkdirSync(path.dirname(file), { recursive: true });
    fs.writeFileSync(file, safeStorage.encryptString(JSON.stringify(obj)), { mode: 0o600 });
    return true;
  }

  return {
    persistent: canEncrypt,
    get(key) {
      if (mem.has(key)) return mem.get(key);
      const v = readAll()[key];
      return v === undefined ? null : v;
    },
    set(key, value) {
      mem.set(key, value);
      const all = readAll(); all[key] = value; writeAll(all);
    },
    remove(key) {
      mem.delete(key);
      const all = readAll();
      if (key in all) { delete all[key]; writeAll(all); }
    }
  };
}

module.exports = { createStore };

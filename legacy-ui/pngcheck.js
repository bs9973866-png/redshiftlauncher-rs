// Validates a Minecraft skin by its real content, never by the file extension.
const SIG = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);

function inspectPng(buf) {
  if (!Buffer.isBuffer(buf) || buf.length < 33) return { ok: false, reason: 'Файл слишком маленький для PNG' };
  if (!buf.subarray(0, 8).equals(SIG)) return { ok: false, reason: 'Это не PNG-файл (неверная сигнатура)' };
  // first chunk must be IHDR: length(4) type(4) data(13)
  if (buf.readUInt32BE(8) !== 13 || buf.toString('ascii', 12, 16) !== 'IHDR') return { ok: false, reason: 'Повреждённый PNG (нет заголовка IHDR)' };
  const width = buf.readUInt32BE(16), height = buf.readUInt32BE(20);
  const colorType = buf[25], interlace = buf[28];
  if (![0, 2, 3, 4, 6].includes(colorType)) return { ok: false, reason: 'Повреждённый PNG (неизвестный тип цвета)' };
  // the file must end with an IEND chunk
  if (buf.toString('ascii', buf.length - 8, buf.length - 4) !== 'IEND') return { ok: false, reason: 'Повреждённый PNG (нет завершающего блока IEND)' };
  return { ok: true, width, height, colorType, interlace };
}

// sizes: [[64,64],[64,32]]
function validateSkin(buf, { sizes, maxBytes }) {
  if (maxBytes && buf.length > maxBytes) return { ok: false, reason: `Файл больше ${(maxBytes / 1024).toFixed(0)} КБ` };
  const info = inspectPng(buf);
  if (!info.ok) return info;
  if (!sizes.some(([w, h]) => w === info.width && h === info.height)) {
    return { ok: false, reason: `Нужен скин ${sizes.map(([w, h]) => `${w}×${h}`).join(' или ')}, а у файла ${info.width}×${info.height}` };
  }
  return info;
}

module.exports = { inspectPng, validateSkin };

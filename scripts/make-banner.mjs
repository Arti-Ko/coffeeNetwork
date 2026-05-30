// Wide README banner for coffeeNetwork: coffee cup with a network-graph "steam"
// on a warm espresso gradient. 1280x420 → .github/assets/banner.png
import { writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import zlib from "node:zlib";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const W = 1280, H = 420;
const b = Buffer.alloc(W * H * 4);

function px(x, y, r, g, bl, a) {
  x |= 0; y |= 0; if (x < 0 || y < 0 || x >= W || y >= H) return;
  const i = (y * W + x) * 4, na = a / 255, ia = 1 - na;
  b[i] = r * na + b[i] * ia; b[i + 1] = g * na + b[i + 1] * ia; b[i + 2] = bl * na + b[i + 2] * ia;
  b[i + 3] = Math.min(255, b[i + 3] + a);
}
function disc(cx, cy, rad, r, g, bl, a = 255) {
  for (let y = cy - rad - 2; y <= cy + rad + 2; y++) for (let x = cx - rad - 2; x <= cx + rad + 2; x++) {
    const d = Math.hypot(x - cx, y - cy); if (d <= rad + 1) px(x, y, r, g, bl, a * Math.max(0, Math.min(1, rad + 0.5 - d)));
  }
}
function line(x0, y0, x1, y1, w, r, g, bl, a = 255) {
  const L = Math.hypot(x1 - x0, y1 - y0), s = Math.ceil(L);
  for (let i = 0; i <= s; i++) { const t = i / s; disc(x0 + (x1 - x0) * t, y0 + (y1 - y0) * t, w / 2, r, g, bl, a); }
}
const lerp = (a, c, t) => a + (c - a) * t;

const c0 = [0x1c, 0x13, 0x0d], c1 = [0x39, 0x25, 0x16];
for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
  const t = Math.max(0, Math.min(1, (x * 0.5 + y * 0.5) / Math.max(W, H)));
  px(x, y, lerp(c0[0], c1[0], t), lerp(c0[1], c1[1], t), lerp(c0[2], c1[2], t), 255);
}
for (let y = 0; y < H; y++) for (let x = 0; x < W; x++) {
  const d = Math.hypot((x - W * 0.42) / W, (y - H * 0.34) / H), l = Math.max(0, 0.3 - d) * 1.4;
  if (l > 0) { const i = (y * W + x) * 4; b[i] = Math.min(255, b[i] + l * 90); b[i + 1] = Math.min(255, b[i + 1] + l * 64); b[i + 2] = Math.min(255, b[i + 2] + l * 38); }
}

// coffee cup
const cx = 470, cupTop = 188, cupBot = 296, topH = 70, botH = 54;
for (let y = cupTop; y <= cupBot; y++) { const t = (y - cupTop) / (cupBot - cupTop); const half = lerp(topH, botH, t); line(cx - half, y, cx + half, y, 2, 245, 244, 240, 255); }
disc(cx - botH + 9, cupBot, 9, 245, 244, 240); disc(cx + botH - 9, cupBot, 9, 245, 244, 240);
for (let y = cupTop - 16; y <= cupTop + 14; y++) for (let x = cx - topH; x <= cx + topH; x++) { const ex = (x - cx) / topH, ey = (y - cupTop) / 16; if (ex * ex + ey * ey <= 1) px(x, y, 60, 38, 22, 255); }
for (let a = -90; a <= 90; a++) { const r = a * Math.PI / 180; disc(cx + topH - 3 + Math.cos(r) * 38, lerp(cupTop, cupBot, 0.42) + Math.sin(r) * 38, 8, 245, 244, 240); }

// network steam
const acc = [233, 161, 60];
const N = [[cx, 150, 12], [cx - 60, 95, 9], [cx + 55, 90, 10], [cx - 10, 55, 14], [cx + 78, 42, 8]];
const L = [[0, 1], [0, 2], [1, 3], [2, 3], [2, 4], [3, 4]];
for (const [i, j] of L) line(N[i][0], N[i][1], N[j][0], N[j][1], 3.4, acc[0], acc[1], acc[2], 230);
for (const [x, y, r] of N) { disc(x, y, r, acc[0], acc[1], acc[2]); disc(x - r * 0.3, y - r * 0.3, r * 0.4, 255, 220, 170, 140); }

function crc(buf) { let c = ~0; for (let i = 0; i < buf.length; i++) { c ^= buf[i]; for (let k = 0; k < 8; k++) c = (c >>> 1) ^ (0xedb88320 & -(c & 1)); } return ~c >>> 0; }
function chunk(t, d) { const l = Buffer.alloc(4); l.writeUInt32BE(d.length); const ty = Buffer.from(t); const cc = Buffer.alloc(4); cc.writeUInt32BE(crc(Buffer.concat([ty, d]))); return Buffer.concat([l, ty, d, cc]); }
const sig = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
const ih = Buffer.alloc(13); ih.writeUInt32BE(W, 0); ih.writeUInt32BE(H, 4); ih[8] = 8; ih[9] = 6;
const raw = Buffer.alloc((W * 4 + 1) * H);
for (let y = 0; y < H; y++) { raw[y * (W * 4 + 1)] = 0; b.copy(raw, y * (W * 4 + 1) + 1, y * W * 4, (y + 1) * W * 4); }
const idat = zlib.deflateSync(raw, { level: 9 });
writeFileSync(path.join(ROOT, ".github/assets/banner.png"), Buffer.concat([sig, chunk("IHDR", ih), chunk("IDAT", idat), chunk("IEND", Buffer.alloc(0))]));
console.log("wrote .github/assets/banner.png");

// Generates assets/sounds/parla-chime.wav: a two-note chime (E5 then A5) with
// a soft exponential decay, 22.05 kHz mono 16-bit PCM, about 1.4 s. Pure
// synthesis from sine waves, so the file is original work (CC0). Run through
// `mise run assets:chime`; the output is deterministic.

const RATE = 22050;
const SECONDS = 1.4;
const NOTES: { start: number; hz: number }[] = [
  { start: 0.0, hz: 659.25 },
  { start: 0.32, hz: 880.0 },
];

const n = Math.round(RATE * SECONDS);
const mix = new Float64Array(n);
for (const { start, hz } of NOTES) {
  const from = Math.round(start * RATE);
  for (let i = from; i < n; i++) {
    const t = (i - from) / RATE;
    const attack = Math.min(1, t / 0.01);
    const decay = Math.exp(-t * 4.5);
    const tone = Math.sin(2 * Math.PI * hz * t) + 0.3 * Math.sin(2 * Math.PI * hz * 2 * t);
    mix[i] += attack * decay * tone;
  }
}
let peak = 0;
for (const v of mix) peak = Math.max(peak, Math.abs(v));

const data = new Uint8Array(44 + n * 2);
const view = new DataView(data.buffer);
const ascii = (at: number, s: string) => {
  for (let i = 0; i < s.length; i++) data[at + i] = s.charCodeAt(i);
};
ascii(0, "RIFF");
view.setUint32(4, 36 + n * 2, true);
ascii(8, "WAVE");
ascii(12, "fmt ");
view.setUint32(16, 16, true);
view.setUint16(20, 1, true);
view.setUint16(22, 1, true);
view.setUint32(24, RATE, true);
view.setUint32(28, RATE * 2, true);
view.setUint16(32, 2, true);
view.setUint16(34, 16, true);
ascii(36, "data");
view.setUint32(40, n * 2, true);
for (let i = 0; i < n; i++) {
  view.setInt16(44 + i * 2, Math.round((mix[i] / peak) * 0.6 * 32767), true);
}

const out = new URL("../assets/sounds/parla-chime.wav", import.meta.url).pathname;
await Bun.write(out, data);
console.log(`wrote ${out} (${data.length} bytes)`);

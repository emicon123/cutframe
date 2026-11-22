/** Drawing math for waveform thumbnails. No React in here. */

/**
 * Reduces interleaved (min, max) bucket pairs to `columns` pairs, each the min and max of the
 * buckets it covers. Columns past the end of a short waveform stay 0 (silence).
 */
export function reducePeaks(peaks: Int8Array, columns: number): Int8Array {
  const out = new Int8Array(Math.max(0, columns) * 2);
  const buckets = Math.floor(peaks.length / 2);
  if (buckets === 0 || columns <= 0) return out;
  for (let c = 0; c < columns; c += 1) {
    const from = Math.floor((c * buckets) / columns);
    const to = Math.max(from + 1, Math.floor(((c + 1) * buckets) / columns));
    let min = 127;
    let max = -127;
    for (let b = from; b < Math.min(to, buckets); b += 1) {
      min = Math.min(min, peaks[2 * b] ?? 0);
      max = Math.max(max, peaks[2 * b + 1] ?? 0);
    }
    out[2 * c] = Math.min(min, max);
    out[2 * c + 1] = max;
  }
  return out;
}

/** Draws a centred min/max waveform filling the canvas, one bar per device pixel column. */
export function drawWaveform(canvas: HTMLCanvasElement, peaks: Int8Array, color: string): void {
  const ctx = canvas.getContext("2d");
  if (!ctx) return;
  const { width, height } = canvas;
  ctx.clearRect(0, 0, width, height);
  const columns = reducePeaks(peaks, width);
  const mid = height / 2;
  const scale = (height / 2 - 1) / 127;
  ctx.fillStyle = color;
  for (let x = 0; x < width; x += 1) {
    const min = columns[2 * x] ?? 0;
    const max = columns[2 * x + 1] ?? 0;
    const top = mid - max * scale;
    const bottom = mid - min * scale;
    ctx.fillRect(x, top, 1, Math.max(1, bottom - top));
  }
}

export function gb(bytes: number, digits = 2): string {
  return `${(bytes / 1e9).toFixed(digits)} GB`;
}

export function bytes(n: number): string {
  if (n >= 1e9) return gb(n);
  if (n >= 1e6) return `${(n / 1e6).toFixed(1)} MB`;
  if (n >= 1e3) return `${(n / 1e3).toFixed(0)} KB`;
  return `${n} B`;
}

export function speed(bps: number): string {
  return `${(bps / 1e6).toFixed(1)} MB/s`;
}

export function eta(remaining: number, bps: number): string {
  if (bps <= 0) return "—";
  const s = Math.round(remaining / bps);
  if (s < 60) return `${s} 秒`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} 分 ${s % 60} 秒`;
  return `${Math.floor(m / 60)} 小时 ${m % 60} 分`;
}

export function mbToGb(mb: number): string {
  return `${(mb / 1024).toFixed(1)} GB`;
}

export const backendName = (b: "vulkan" | "cpu") => (b === "vulkan" ? "GPU" : "CPU");

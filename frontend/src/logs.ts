/** 将结构化访问记录转换为可读字段，保留不能解析的原始文本。 */
export interface LogEntry {
  id: string;
  time: string;
  source: string;
  request: string;
  status: string;
  detail: string;
}
/** 将日志中的时间转换为本地可读格式，无法解析时保留原值。 */
function time(value: unknown, locale: string): string {
  if (typeof value !== "string") return "—";
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : date.toLocaleString(locale, { hour12: false });
}
/** 访问记录逐行转换；未知结构完整保留，便于排查。 */
export function logEntries(lines: string[], locale = "zh-CN"): LogEntry[] {
  return lines.map((line, index) => {
    const empty = {
      id: String(index),
      time: "—",
      source: "—",
      request: "—",
      status: "—",
      detail: "",
    };
    let value: unknown;
    try {
      value = JSON.parse(line);
    } catch {
      return { ...empty, detail: line };
    }
    if (!value || typeof value !== "object" || Array.isArray(value))
      return { ...empty, detail: typeof value === "string" ? value : line };
    const v = value as Record<string, unknown>;
    if (!("method" in v) && !("remote" in v)) return { ...empty, detail: line };
    return {
      ...empty,
      time: time(v.time, locale),
      source: String(v.remote ?? "—"),
      request: `${v.method ?? ""} ${v.uri ?? ""}`.trim() || "—",
      status: String(v.status ?? "—"),
      detail: String(v.host ?? ""),
    };
  });
}

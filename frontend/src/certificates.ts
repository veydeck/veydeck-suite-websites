/** 证书有效期状态只表示时间有效性。 */
export function certificateStatus(
  certificate: { notBefore: number; notAfter: number },
  now = Date.now(),
) {
  if (now < certificate.notBefore)
    return { label: "尚未生效", type: "warning" as const };
  if (now >= certificate.notAfter)
    return { label: "已过期", type: "danger" as const };
  if (certificate.notAfter - now <= 30 * 86400000)
    return { label: "即将过期", type: "warning" as const };
  return { label: "有效", type: "success" as const };
}
/** 按界面语言及浏览器时区显示证书 UTC 时间，显式保留时区标识。 */
export function formatCertificateTime(
  value: number,
  locale = "zh-CN",
  timeZone?: string,
) {
  if (!Number.isFinite(value)) return "—";
  // 标准未定义到期日固定显示 UTC，避免东八区换算后进入 10000 年。
  if (value === 253402300799000) return "9999-12-31 23:59:59 UTC";
  return new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hourCycle: "h23",
    timeZoneName: "short",
    ...(timeZone ? { timeZone } : {}),
  }).format(new Date(value));
}

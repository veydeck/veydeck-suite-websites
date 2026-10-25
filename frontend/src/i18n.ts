/** 套件语言、插值及结构化错误的统一解析。 */
import { ref } from "vue";
import messages from "./locales/messages.json";
import errors from "./locales/errors.json";
export type Locale = "zh-CN" | "en-US";
export const locale = ref<Locale>("zh-CN");
export type MessageParams = Record<string, unknown>;
interface ApiErrorPayload {
  code?: string;
  params?: MessageParams;
  error?: string;
}
/** 数字只用于展示，输入框与 Nginx 配置值维持机器可解析的原值。 */
export function formatNumber(value: number) {
  return new Intl.NumberFormat(locale.value).format(value);
}
/** 嵌套参数显式区分业务文案、错误原因与原始诊断。 */
function parameter(value: unknown, depth: number): string {
  if (depth > 5) return "…";
  if (typeof value === "number") return formatNumber(value);
  if (value && typeof value === "object") {
    if ("key" in value && typeof value.key === "string")
      return translate(
        value.key,
        "params" in value ? (value.params as MessageParams) : {},
        depth + 1,
      );
    if ("code" in value)
      return apiMessage(value as ApiErrorPayload, 400, depth + 1);
  }
  return value == null ? "" : String(value);
}
/** 替换受控占位符，不将用户输入解释成模板或 HTML。 */
function interpolate(
  message: string,
  params: MessageParams,
  depth: number,
): string {
  return message.replace(/\{([a-zA-Z]\w*)\}/g, (placeholder, key: string) =>
    Object.hasOwn(params, key) ? parameter(params[key], depth) : placeholder,
  );
}
/** 按当前语言读取文案；中文键也是中文回退值。 */
function translate(key: string, params: MessageParams, depth: number): string {
  const entry = Object.hasOwn(messages, key)
    ? messages[key as keyof typeof messages]
    : undefined;
  return interpolate(
    locale.value === "en-US" ? (entry ?? key) : key,
    params,
    depth,
  );
}
/** 翻译界面文案并插入受控参数。 */
export function t(key: string, params: MessageParams = {}): string {
  return translate(key, params, 0);
}
/** 错误码独立于服务端诊断文字，未知错误保留 HTTP 状态。 */
function apiMessage(
  payload: ApiErrorPayload,
  status: number,
  depth = 0,
): string {
  const code =
    (typeof payload.code === "string" ? payload.code : undefined) ??
    (status === 401
      ? "UNAUTHORIZED"
      : status === 422 || status === 415
        ? "INVALID_REQUEST"
        : "HTTP_ERROR");
  const entry = Object.hasOwn(errors, code)
    ? errors[code as keyof typeof errors]
    : undefined;
  if (!payload.code && payload.error && ![401, 415, 422].includes(status))
    return payload.error;
  return interpolate(
    entry?.[locale.value] ?? errors.HTTP_ERROR[locale.value],
    { status, ...payload.params },
    depth,
  );
}
/** 记录错误码和原始参数，显示时才翻译以支持实时语言切换。 */
export class ApiError extends Error {
  constructor(
    readonly payload: ApiErrorPayload,
    readonly status: number,
  ) {
    super(payload.error || payload.code || String(status));
    this.name = "ApiError";
  }
}
/** 前端校验也保存文案键，避免语言切换后残留旧语言。 */
export class LocalizedError extends Error {
  constructor(
    readonly key: string,
    readonly params: MessageParams = {},
  ) {
    super(key);
    this.name = "LocalizedError";
  }
}
/** 所有界面错误在渲染时解析，技术日志和外部诊断保留原文。 */
export function errorMessage(value: unknown): string {
  if (value instanceof ApiError) return apiMessage(value.payload, value.status);
  if (value instanceof LocalizedError) return t(value.key, value.params);
  if (value instanceof TypeError) return errors.NETWORK_ERROR[locale.value];
  if (value instanceof Error) return t(value.message);
  return typeof value === "string" ? t(value) : "";
}

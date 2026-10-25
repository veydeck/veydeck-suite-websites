/** API 与二进制下载共用结构化错误处理，相对路径兼容平台代理。 */
import { ApiError, locale } from "./i18n";
/** 业务错误保留原始错误码和参数，显示时使用当前语言。 */
export async function checkResponse(response: Response): Promise<Response> {
  if (!response.ok) {
    const payload = await response.json().catch(() => ({}));
    throw new ApiError(
      payload && typeof payload === "object" ? payload : {},
      response.status,
    );
  }
  return response;
}
/** 请求显式携带语言；服务端返回的业务错误码与语言无关。 */
export async function apiResponse(
  path: string,
  options: RequestInit = {},
): Promise<Response> {
  const headers = new Headers(options.headers);
  headers.set("Accept-Language", locale.value);
  return checkResponse(await fetch(`api/${path}`, { ...options, headers }));
}
/** 相对 URL 兼容平台代理路径。 */
export async function api<T>(
  path: string,
  method = "GET",
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const response = await apiResponse(path, {
    method,
    signal,
    headers: body === undefined ? {} : { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  return response.status === 204
    ? (undefined as T)
    : ((await response.json()) as T);
}

/** 日志来源与类型变化时自动读取，取消旧请求并保证仅最新结果可写入。 */
import { onBeforeUnmount, ref, watch, type Ref } from "vue";
import { api } from "./api";
export function useLogs(source: Ref<string | null>, kind: Ref<string>) {
  const lines = ref<string[]>([]),
    loading = ref(false),
    error = ref<unknown>("");
  let requestId = 0,
    controller: AbortController | undefined;
  /** 清空旧内容后读取当前来源，失败和完成状态同样遵循最新请求优先。 */
  async function refresh() {
    const id = ++requestId,
      site = source.value,
      selectedKind = kind.value;
    controller?.abort();
    lines.value = [];
    error.value = "";
    loading.value = false;
    if (site === null) return;
    controller = new AbortController();
    loading.value = true;
    try {
      const result = await api<string[]>(
        `logs?kind=${selectedKind}${site ? `&site=${encodeURIComponent(site)}` : ""}`,
        "GET",
        undefined,
        controller.signal,
      );
      if (id === requestId) lines.value = result;
    } catch (e) {
      if (
        id === requestId &&
        !(e instanceof DOMException && e.name === "AbortError")
      )
        error.value = e;
    } finally {
      if (id === requestId) loading.value = false;
    }
  }
  watch([source, kind], () => void refresh(), { immediate: true });
  onBeforeUnmount(() => {
    ++requestId;
    controller?.abort();
  });
  return { lines, loading, error, refresh };
}

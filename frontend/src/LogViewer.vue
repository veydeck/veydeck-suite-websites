<script setup lang="ts">
/** 访问日志按字段展示，服务错误日志保留文本。 */
import { computed } from "vue";
import { VeyAlert, VeyTable, VeyTag } from "@veydeck/vue";
import { logEntries } from "./logs";
import { locale, t, errorMessage } from "./i18n";
const props = defineProps<{
  kind: string;
  lines: string[];
  loading: boolean;
  error: unknown;
}>();
const entries = computed(() => logEntries(props.lines, locale.value));
const columns = computed(() => [
  { prop: "time", label: t("时间"), minWidth: 165 },
  { prop: "source", label: t("来源 IP"), minWidth: 120 },
  { slot: "request", label: t("请求"), minWidth: 180 },
  { slot: "status", label: t("状态"), width: 90 },
  { slot: "detail", label: t("域名 / 详情"), minWidth: 160 },
]);
</script>
<template>
  <div class="editor" data-ui="log-viewer" :aria-busy="loading">
    <VeyAlert v-if="error" type="error" :description="errorMessage(error)" />
    <div v-if="loading" class="muted" role="status">
      {{ t("正在读取日志…") }}
    </div>
    <pre v-else-if="kind === 'error'" class="log" data-ui="log-text">{{
      lines.join("\n") || t("暂无日志")
    }}</pre>
    <VeyTable
      v-else
      :data="entries"
      row-key="id"
      :columns="columns"
      :empty-text="t('暂无日志')"
      data-ui="log-table"
    >
      <template #request="{ row }"
        ><span class="mono log-value">{{ row.request }}</span></template
      >
      <template #status="{ row }"
        ><VeyTag :type="Number(row.status) >= 400 ? 'danger' : 'default'">{{
          row.status
        }}</VeyTag></template
      >
      <template #detail="{ row }"
        ><span class="log-value">{{ row.detail }}</span></template
      >
    </VeyTable>
  </div>
</template>

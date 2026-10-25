<script setup lang="ts" generic="T extends object">
/** 按可用高度分页，固定表头、行高和底部分页区。 */
import {
  computed,
  nextTick,
  onBeforeUnmount,
  onMounted,
  ref,
  watch,
} from "vue";
import { VeyPagination, VeyTable, type VeyTableColumn } from "@veydeck/vue";
import { t } from "./i18n";
const props = defineProps<{
  data: T[];
  columns: VeyTableColumn[];
  rowKey: string;
  label: string;
}>();
defineSlots<{
  [name: string]: (scope: { row: T; index: number }) => unknown;
}>();
const region = ref<HTMLElement | null>(null);
const page = ref(1),
  pageSize = ref(1),
  compact = ref(false);
const totalPages = computed(() =>
  Math.max(1, Math.ceil(props.data.length / pageSize.value)),
);
const visibleRows = computed(() =>
  props.data.slice(
    (page.value - 1) * pageSize.value,
    page.value * pageSize.value,
  ),
);
/** 固定布局显式落实列的最小宽度，避免英文操作按钮被压缩裁剪。 */
const layoutColumns = computed(() =>
  props.columns.map((column) => ({
    ...column,
    width: column.width ?? column.minWidth,
  })),
);
const minimumWidth = computed(() =>
  props.columns.reduce(
    (sum, col) => sum + (Number(col.width ?? col.minWidth) || 160),
    0,
  ),
);
let observer: ResizeObserver | undefined,
  frame = 0;
/** 高度使用真实表头与滚动条占用，保持当前首条数据所在页。 */
function measure() {
  const wrapper = region.value?.querySelector<HTMLElement>(".vl-table-wrapper");
  const header = wrapper?.querySelector("thead");
  if (!wrapper || !header || wrapper.clientHeight === 0) return;
  const size = Math.max(
    1,
    Math.floor(
      (wrapper.clientHeight - header.getBoundingClientRect().height - 1) / 52,
    ),
  );
  if (size !== pageSize.value) {
    const first = (page.value - 1) * pageSize.value;
    pageSize.value = size;
    page.value = Math.min(totalPages.value, Math.floor(first / size) + 1);
  }
  compact.value = wrapper.clientWidth < 500;
}
/** 合并同一帧内的尺寸更新，避免布局观察器循环触发。 */
function scheduleMeasure() {
  cancelAnimationFrame(frame);
  frame = requestAnimationFrame(measure);
}
/** 删除最后一页数据后自动回到最后一个有效页。 */
watch(
  totalPages,
  (total) => {
    page.value = Math.min(page.value, total);
  },
  { flush: "sync" },
);
watch(
  () => props.data.length,
  async () => {
    await nextTick();
    scheduleMeasure();
  },
);
onMounted(() => {
  observer = new ResizeObserver(scheduleMeasure);
  if (region.value) observer.observe(region.value);
  const wrapper = region.value?.querySelector(".vl-table-wrapper");
  if (wrapper) observer.observe(wrapper);
  scheduleMeasure();
});
onBeforeUnmount(() => {
  observer?.disconnect();
  cancelAnimationFrame(frame);
});
</script>
<template>
  <div
    class="paginated-table"
    :style="{ '--paged-table-min-width': `${minimumWidth}px` }"
  >
    <div ref="region" class="paginated-table-region" data-slot="table-region">
      <VeyTable
        :data="visibleRows"
        :columns="layoutColumns"
        :row-key="rowKey"
        :empty-text="t('无数据')"
      >
        <template v-for="(_, name) in $slots" #[name]="scope"
          ><slot :name="name" v-bind="scope"
        /></template>
      </VeyTable>
    </div>
    <div
      class="table-pagination"
      data-ui="table-pagination"
      :data-page="page"
      :data-page-size="pageSize"
      :data-total-pages="totalPages"
    >
      <span class="muted" aria-live="polite">{{
        t("共 {total} 条 · 第 {page}/{pages} 页 · 每页 {size} 条", {
          total: data.length,
          page,
          pages: totalPages,
          size: pageSize,
        })
      }}</span>
      <VeyPagination
        :current-page="page"
        :total-pages="totalPages"
        :max-visible-buttons="compact ? 1 : 5"
        :aria-label="t('{label}分页', { label })"
        :previous-label="t('上一页')"
        :next-label="t('下一页')"
        @page-change="page = $event"
      />
    </div>
  </div>
</template>
<style scoped>
.paginated-table {
  display: flex;
  flex-direction: column;
  flex: 1;
  min-width: 0;
  min-height: 0;
  gap: var(--vdl-space-3);
}
.paginated-table-region {
  flex: 1;
  min-height: 0;
  min-width: 0;
}
.table-pagination {
  display: flex;
  flex: none;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: var(--vdl-space-2);
}
.paginated-table :deep(.vl-table-wrapper) {
  overflow-y: hidden;
}
.paginated-table :deep(.vl-table) {
  table-layout: fixed;
  min-width: var(--paged-table-min-width);
}
.paginated-table :deep(.vl-table-cell) {
  height: 52px;
  box-sizing: border-box;
  padding-block: 0;
}
.paginated-table :deep(.vl-cell),
.paginated-table :deep(.log-value) {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.paginated-table :deep(.log-value) {
  display: block;
}
</style>

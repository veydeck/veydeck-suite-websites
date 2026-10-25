<script setup lang="ts">
/** 全局性能配置编辑器，只展示已确认生效的服务端事实。 */
import { onMounted, ref } from "vue";
import { VeyAlert, VeyButton, VeyInput, VeySwitch } from "@veydeck/vue";
import { api } from "./api";
import { t, errorMessage, LocalizedError } from "./i18n";
import type { Performance } from "./types";
const value = ref<Performance | null>(null);
const busy = ref(false),
  error = ref<unknown>(""),
  notice = ref("");
const fields = [
  {
    key: "serverNamesHashBucketSize",
    directive: "server_names_hash_bucket_size",
    hint: "域名哈希桶大小（字节），32～65536，必须为 2 的幂",
    min: 32,
    max: 65536,
  },
  {
    key: "clientHeaderBufferSizeKb",
    directive: "client_header_buffer_size",
    hint: "请求头缓冲区（KB），1～1024",
    min: 1,
    max: 1024,
  },
  {
    key: "clientMaxBodySizeMb",
    directive: "client_max_body_size",
    hint: "请求体大小上限（MB），0 表示不限制，最大 1048576",
    min: 0,
    max: 1048576,
  },
  {
    key: "keepaliveTimeout",
    directive: "keepalive_timeout",
    hint: "空闲连接超时（秒），0 表示禁用长连接，最大 86400",
    min: 0,
    max: 86400,
  },
  {
    key: "gzipMinLength",
    directive: "gzip_min_length",
    hint: "最小压缩响应（字节），1024 字节 = 1 KB，最大 1073741824",
    min: 0,
    max: 1073741824,
  },
  {
    key: "gzipCompLevel",
    directive: "gzip_comp_level",
    hint: "压缩级别，1～9；越高消耗的 CPU 越多",
    min: 1,
    max: 9,
  },
] as const;
/** 获取服务端生效值，刷新会放弃尚未保存的输入。 */
async function load() {
  busy.value = true;
  error.value = "";
  notice.value = "";
  try {
    value.value = await api<Performance>("settings/performance");
  } catch (e) {
    error.value = e;
  } finally {
    busy.value = false;
  }
}
/** 数字输入先显式校验，再提交修订号防止覆盖其他会话。 */
async function save() {
  if (!value.value) return;
  busy.value = true;
  error.value = "";
  notice.value = "";
  try {
    const candidate = { ...value.value };
    for (const field of fields) {
      const raw = candidate[field.key];
      const number = Number(raw);
      if (
        String(raw).trim() === "" ||
        !Number.isInteger(number) ||
        number < field.min ||
        number > field.max
      )
        throw new LocalizedError("参数 {directive}：{hint}", {
          directive: field.directive,
          hint: { key: field.hint },
        });
      candidate[field.key] = number;
    }
    value.value = await api<Performance>(
      "settings/performance",
      "PUT",
      candidate,
    );
    notice.value = "已保存并重载";
  } catch (e) {
    error.value = e;
  } finally {
    busy.value = false;
  }
}
onMounted(load);
</script>
<template>
  <div class="editor" data-ui="performance-settings" :aria-busy="busy">
    <div class="section-toolbar">
      <h2 class="page-title">{{ t("性能") }}</h2>
      <VeyButton :disabled="busy" @click="load">{{ t("刷新") }}</VeyButton>
    </div>
    <p class="muted">
      {{
        t(
          "修改 Nginx 全局配置，对所有网站生效；网站高级配置中显式设置的同名指令优先生效。",
        )
      }}
    </p>
    <VeyAlert v-if="error" type="error" :description="errorMessage(error)" />
    <VeyAlert v-if="notice" type="success" :description="t(notice)" />
    <template v-if="value">
      <div class="grid">
        <label v-for="field in fields" :key="field.key"
          >{{ field.directive
          }}<VeyInput
            v-model="value[field.key]"
            type="number"
            :min="field.min"
            :max="field.max"
            :disabled="busy"
            :data-ui="`performance-${field.key}`"
          /><small class="muted">{{ t(field.hint) }}</small></label
        >
        <label
          >gzip<VeySwitch
            v-model="value.gzip"
            :disabled="busy"
            :aria-label="t('启用 gzip 压缩')"
            data-ui="performance-gzip"
          /><small class="muted">{{
            t("启用响应压缩；压缩类型遵循 Nginx 默认规则和网站配置。")
          }}</small></label
        >
      </div>
      <div class="actions">
        <VeyButton
          type="primary"
          :disabled="busy"
          data-ui="save-performance"
          @click="save"
          >{{ t("保存并重载") }}</VeyButton
        >
      </div>
    </template>
  </div>
</template>

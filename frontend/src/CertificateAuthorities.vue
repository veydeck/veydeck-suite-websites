<script setup lang="ts">
/** 实例私有根证书的创建、查看、下载和网站证书签发。 */
import { computed, onMounted, onBeforeUnmount, ref } from "vue";
import {
  VeyAlert,
  VeyButton,
  VeyDescriptions,
  VeyDialog,
  VeyInput,
  VeySelect,
  VeyTag,
} from "@veydeck/vue";
import { api, apiResponse } from "./api";
import { errorMessage, LocalizedError } from "./i18n";
import PaginatedTable from "./PaginatedTable.vue";
import { locale, t } from "./bridge";
import { certificateStatus, formatCertificateTime } from "./certificates";
import type { Certificate, CertificateAuthority } from "./types";
const emit = defineEmits<{ issued: [certificate: Certificate] }>();
const roots = ref<CertificateAuthority[]>([]),
  busy = ref(false),
  error = ref<unknown>(""),
  notice = ref("");
const createDialog = ref(false),
  detail = ref<CertificateAuthority | null>(null),
  signing = ref<CertificateAuthority | null>(null);
const rootInput = ref({
  name: "",
  organization: "Veydeck",
  algorithm: "ec256",
  years: 10,
});
const issueInput = ref({ domains: "", algorithm: "ec256", years: 10 });
const clock = ref(Date.now());
let timer: ReturnType<typeof setInterval> | undefined;
const algorithms = [
  { value: "ec256", label: "EC 256" },
  { value: "rsa2048", label: "RSA 2048" },
  { value: "rsa4096", label: "RSA 4096" },
];
/** 所有异步操作统一处理错误与重复提交。 */
async function run(action: () => Promise<void>) {
  if (busy.value) return;
  busy.value = true;
  error.value = "";
  notice.value = "";
  try {
    await action();
  } catch (e) {
    error.value = e;
  } finally {
    busy.value = false;
  }
}
/** 从实例管理接口读取根证书公开信息。 */
async function load() {
  roots.value = await api<CertificateAuthority[]>("certificate-authorities");
}
/** 时间跟随主控语言和浏览器时区。 */
function date(value: number) {
  return formatCertificateTime(value, locale.value);
}
/** 根证书过期后界面禁用签发，服务端仍独立检查有效期。 */
function status(value: CertificateAuthority) {
  return certificateStatus(value, clock.value);
}
/** 打开空白创建表单，默认使用 EC 256 与十年有效期。 */
function prepareCreate() {
  error.value = "";
  rootInput.value = {
    name: "",
    organization: "Veydeck",
    algorithm: "ec256",
    years: 10,
  };
  createDialog.value = true;
}
/** 创建新根证书不替换默认根或影响已有证书。 */
async function create() {
  await api("certificate-authorities", "POST", {
    ...rootInput.value,
    years: Number(rootInput.value.years),
  });
  createDialog.value = false;
  await load();
  notice.value = "根证书已创建";
}
/** 从指定根证书发起签发，不要求先创建新的根。 */
function prepareIssue(root: CertificateAuthority) {
  error.value = "";
  issueInput.value = { domains: "", algorithm: "ec256", years: 10 };
  signing.value = root;
}
/** 签发成功后回到网站证书列表，供用户直接选择配置网站。 */
async function issue() {
  if (!signing.value) return;
  const domains = issueInput.value.domains.split(/[\s,，]+/).filter(Boolean);
  if (!domains.length) throw new LocalizedError("请填写域名或 IP");
  const value = await api<Certificate>(
    `certificate-authorities/${signing.value.id}/issue`,
    "POST",
    {
      domains,
      algorithm: issueInput.value.algorithm,
      years: Number(issueInput.value.years),
    },
  );
  signing.value = null;
  emit("issued", value);
}
/** 仅下载公开 PEM，私钥不经此接口传输。 */
async function download(root: CertificateAuthority) {
  const response = await apiResponse(
    `certificate-authorities/${root.id}/download`,
  );
  const url = URL.createObjectURL(await response.blob()),
    link = document.createElement("a");
  link.href = url;
  link.download = `veydeck-root-${root.id}.pem`;
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
const details = computed(() => {
  const root = detail.value;
  if (!root) return [];
  return [
    { label: t("名称"), value: root.name },
    { label: t("组织"), value: root.organization || "—" },
    { label: t("状态"), value: t(status(root).label) },
    { label: t("创建时间"), value: date(root.createdAt) },
    { label: t("生效时间"), value: date(root.notBefore) },
    { label: t("过期时间"), value: date(root.notAfter) },
    { label: t("主题"), value: root.subject },
    { label: t("颁发者"), value: root.issuer },
    { label: t("序列号"), value: root.serialNumber },
    { label: t("SHA-256 指纹"), value: root.fingerprintSha256 },
    { label: t("公钥类型"), value: root.publicKey },
    { label: t("签名算法"), value: root.signatureAlgorithm },
  ];
});
onMounted(() => {
  void run(load);
  timer = setInterval(() => {
    clock.value = Date.now();
  }, 60000);
});
onBeforeUnmount(() => clearInterval(timer));
</script>
<template>
  <div
    class="editor list-page"
    data-page="certificate-authorities"
    :aria-busy="busy"
  >
    <VeyAlert
      v-if="error"
      type="error"
      :description="errorMessage(error)"
      closable
      @close="error = ''"
    />
    <VeyAlert
      v-if="notice"
      type="success"
      :description="t(notice)"
      closable
      @close="notice = ''"
    />
    <div class="section-toolbar">
      <VeyButton type="primary" :disabled="busy" @click="prepareCreate">{{
        t("创建根证书")
      }}</VeyButton
      ><VeyButton :disabled="busy" @click="run(load)">{{
        t("刷新")
      }}</VeyButton>
    </div>
    <p class="muted">
      {{
        t(
          "选择根证书即可签发网站证书；客户端安装并信任相应根证书后，才能信任其签发的网站证书。",
        )
      }}
    </p>
    <div v-if="busy && !roots.length" role="status">
      {{ t("正在读取根证书…") }}
    </div>
    <PaginatedTable
      :label="t('自签证书')"
      :data="roots"
      row-key="id"
      data-ui="authorities-table"
      :columns="[
        { slot: 'name', label: t('名称'), minWidth: 190, align: 'left' },
        {
          prop: 'publicKey',
          label: t('密钥算法'),
          minWidth: 120,
          align: 'left',
        },
        { slot: 'status', label: t('状态'), width: 90, align: 'center' },
        { slot: 'created', label: t('创建时间'), minWidth: 200, align: 'left' },
        { slot: 'expiry', label: t('过期时间'), minWidth: 200, align: 'left' },
        {
          slot: 'actions',
          label: t('操作'),
          minWidth: locale === 'en-US' ? 310 : 210,
          align: 'center',
        },
      ]"
    >
      <template #name="{ row }"
        ><span class="log-value">{{ row.name }}</span>
      </template>
      <template #status="{ row }"
        ><VeyTag :type="status(row).type">{{
          t(status(row).label)
        }}</VeyTag></template
      >
      <template #created="{ row }">{{ date(row.createdAt) }}</template
      ><template #expiry="{ row }">{{ date(row.notAfter) }}</template>
      <template #actions="{ row }"
        ><div class="actions table-actions" data-ui="table-actions">
          <VeyButton
            size="small"
            :disabled="busy || clock < row.notBefore || clock >= row.notAfter"
            @click="prepareIssue(row)"
            >{{ t("签发证书") }}</VeyButton
          ><VeyButton size="small" @click="detail = row">{{
            t("详情")
          }}</VeyButton
          ><VeyButton
            size="small"
            :disabled="busy"
            @click="run(() => download(row))"
            >{{ t("下载") }}</VeyButton
          >
        </div></template
      >
    </PaginatedTable>
    <VeyDialog
      :visible="createDialog"
      :title="t('创建根证书')"
      width="620px"
      @close="!busy && (createDialog = false)"
    >
      <div class="editor" data-ui="authority-create">
        <label
          >{{ t("根证书名称")
          }}<VeyInput v-model="rootInput.name" :maxlength="64" /></label
        ><label
          >{{ t("组织")
          }}<VeyInput
            v-model="rootInput.organization"
            :maxlength="128" /></label
        ><label
          >{{ t("密钥算法")
          }}<VeySelect
            :placeholder="t('请选择')"
            v-model="rootInput.algorithm"
            :options="algorithms"
            data-ui="authority-algorithm" /></label
        ><label
          >{{ t("有效期（年）")
          }}<VeyInput
            v-model.number="rootInput.years"
            type="number"
            :min="1"
            :max="99" /></label
        ><VeyAlert
          v-if="error"
          type="error"
          :description="errorMessage(error)"
        />
      </div>
      <template #footer
        ><VeyButton :disabled="busy" @click="createDialog = false">{{
          t("取消")
        }}</VeyButton
        ><VeyButton type="primary" :disabled="busy" @click="run(create)">{{
          t("确认创建")
        }}</VeyButton></template
      >
    </VeyDialog>
    <VeyDialog
      :visible="!!signing"
      :title="t('签发网站证书')"
      width="660px"
      @close="!busy && (signing = null)"
    >
      <div class="editor" data-ui="authority-issue">
        <span>{{ t("根证书：{name}", { name: signing?.name }) }}</span
        ><label
          >{{ t("域名或 IP")
          }}<VeyInput
            v-model="issueInput.domains"
            type="textarea"
            :rows="5"
            :placeholder="t('每行一项，支持通配域名、IPv4 和 IPv6')" /></label
        ><label
          >{{ t("密钥算法")
          }}<VeySelect
            :placeholder="t('请选择')"
            v-model="issueInput.algorithm"
            :options="algorithms"
            data-ui="issue-algorithm" /></label
        ><label
          >{{ t("有效期（年）")
          }}<VeyInput
            v-model.number="issueInput.years"
            type="number"
            :min="1"
            :max="99"
        /></label>
        <p class="muted">{{ t("实际有效期不超过所选根证书的到期时间。") }}</p>
        <VeyAlert
          v-if="error"
          type="error"
          :description="errorMessage(error)"
        />
      </div>
      <template #footer
        ><VeyButton :disabled="busy" @click="signing = null">{{
          t("取消")
        }}</VeyButton
        ><VeyButton type="primary" :disabled="busy" @click="run(issue)">{{
          t("确认签发")
        }}</VeyButton></template
      >
    </VeyDialog>
    <VeyDialog
      :visible="!!detail"
      :title="t('根证书详情')"
      width="800px"
      @close="detail = null"
      ><div class="log-value" data-ui="authority-detail">
        <VeyDescriptions :items="details" :column="1" />
      </div>
      <template #footer
        ><VeyButton @click="detail = null">{{ t("关闭") }}</VeyButton></template
      ></VeyDialog
    >
  </div>
</template>

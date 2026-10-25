<script setup lang="ts">
/** 网站即时配置、目录导航与分站日志工作台。 */
import { computed, onMounted, onBeforeUnmount, ref, watch } from "vue";
import {
  VeyAlert,
  VeyButton,
  VeyCheckbox,
  VeyDialog,
  VeyDescriptions,
  VeyGlassProvider,
  VeyIcon,
  VeyInput,
  VeyMenu,
  VeySelect,
  VeySwitch,
  VeyTabs,
  VeyTag,
} from "@veydeck/vue";
import { api, apiResponse } from "./api";
import { errorMessage, LocalizedError } from "./i18n";
import { bridge, locale, t } from "./bridge";
import type { Certificate, Site, Snapshot } from "./types";
import { certificateStatus, formatCertificateTime } from "./certificates";
import CertificateAuthorities from "./CertificateAuthorities.vue";
import LogViewer from "./LogViewer.vue";
import PaginatedTable from "./PaginatedTable.vue";
import PerformanceSettings from "./PerformanceSettings.vue";
import { useLogs } from "./useLogs";
const snapshot = ref<Snapshot>({
  sites: [],
  certificates: [],
  hostDataDir: "",
  runtimeConnected: false,
});
const tab = ref("sites"),
  busy = ref(false),
  error = ref<unknown>(""),
  notice = ref("");
const glass = ref(bridge.getThemeSnapshot().glassEnabled);
const unsubscribe = bridge.subscribeTheme((v) => {
  glass.value = v.glassEnabled;
});
const menuItems = computed(() => [
  {
    key: "websites",
    label: t("网站"),
    children: [{ key: "sites", label: t("网站列表") }],
  },
  {
    key: "certificates",
    label: t("证书"),
    children: [
      { key: "certs", label: t("证书列表") },
      { key: "authorities", label: t("自签证书") },
      { key: "upload", label: t("上传证书") },
    ],
  },
  {
    key: "settings",
    label: t("设置"),
    children: [
      { key: "performance", label: t("性能") },
      { key: "logs", label: t("日志") },
      { key: "backup", label: t("备份") },
    ],
  },
]);
const editing = ref<Site | null>(null),
  domains = ref(""),
  siteTab = ref("settings");
const codeEdited = ref(false);
/** 首个域名变化时更新默认代号，手动编辑后保持用户输入。 */
watch(
  domains,
  (value) => {
    if (!editing.value || editing.value.id || codeEdited.value) return;
    const first = value.split(/[\s,]+/).find(Boolean) ?? "";
    try {
      editing.value.directory = first
        ? new URL(`http://${first.replace(/\.+$/, "")}/`).hostname.toLowerCase()
        : "";
    } catch {
      editing.value.directory = first;
    }
  },
  { flush: "sync" },
);
const siteTabs = computed(() => [
  { name: "settings", label: t("基本配置") },
  ...(editing.value?.id
    ? [
        { name: "directory", label: t("网站目录") },
        { name: "logs", label: t("网站日志") },
      ]
    : []),
]);
const certificates = computed(() => [
  { value: "", label: t("无证书（HTTP）") },
  ...snapshot.value.certificates.map((c) => ({
    value: c.id,
    label: `${certificateDomain(c)} · ${certificateTime(c.notAfter)} · ${c.fingerprintSha256.slice(0, 11)}`,
  })),
]);
const deletingCertificate = ref<Certificate | null>(null);
const certificateDetail = ref<Certificate | null>(null);
/** 根证书签发完成后刷新网站证书并回到列表。 */
async function issuedCertificate() {
  await refresh();
  tab.value = "certs";
  notice.value = "网站证书已签发，可在网站配置中选择";
}
const certificateNow = ref(Date.now());
let certificateClock: ReturnType<typeof setInterval> | undefined;
/** 缺少网站身份的证书使用指纹识别，不要求手填名称。 */
function certificateDomain(value: Certificate) {
  return (
    value.domain ||
    `${t("未提供域名")} · ${value.fingerprintSha256.slice(0, 11)}`
  );
}
/** 使用当前界面语言及浏览器本地时区。 */
function certificateTime(value: number) {
  return formatCertificateTime(value, locale.value);
}
/** 每分钟更新有效期状态，页面无需重新加载。 */
function status(value: Certificate) {
  return certificateStatus(value, certificateNow.value);
}
const certificateDetails = computed(() => {
  const value = certificateDetail.value;
  if (!value) return [];
  return [
    { label: t("主域名"), value: certificateDomain(value) },
    {
      label: t("其他域名"),
      value: value.domains.filter((d) => d !== value.domain).join("、") || "—",
    },
    { label: t("状态"), value: t(status(value).label) },
    { label: t("颁发组织"), value: value.issuerOrganization || "—" },
    { label: t("生效时间"), value: certificateTime(value.notBefore) },
    { label: t("过期时间"), value: certificateTime(value.notAfter) },
    { label: t("主题"), value: value.subject || "—" },
    { label: t("颁发者"), value: value.issuer || "—" },
    { label: t("序列号"), value: value.serialNumber },
    { label: t("SHA-256 指纹"), value: value.fingerprintSha256 },
    { label: t("签名算法"), value: value.signatureAlgorithm },
    { label: t("公钥类型"), value: value.publicKey },
    { label: t("证书链长度"), value: value.chainLength },
  ];
});
const certificate = ref({ certificate: "", key: "" });
const deleting = ref<Site | null>(null),
  deleteFiles = ref(false);
const logKind = ref("access");
const logSource = computed(() =>
  editing.value
    ? siteTab.value === "logs"
      ? editing.value.id || null
      : null
    : tab.value === "logs"
      ? ""
      : null,
);
const {
  lines: logLines,
  loading: logsLoading,
  error: logsError,
  refresh: readLogs,
} = useLogs(logSource, logKind);
/** 构造网站在节点上的实际目录。 */
function directoryPath(site: Site) {
  return `${snapshot.value.hostDataDir}/sites/${site.directory}/index`;
}
/** 服务确认生效后刷新开关事实，失败时保持原状态。 */
async function toggleSite(site: Site, enabled: boolean) {
  const saved = await api<Site>(`sites/${site.id}`, "PUT", {
    ...site,
    enabled,
  });
  snapshot.value.sites = snapshot.value.sites.map((current) =>
    current.id === saved.id ? saved : current,
  );
}
const sitePath = computed(
  () => `${snapshot.value.hostDataDir}/sites/${editing.value?.directory}/index`,
);
/** 操作期间禁用重复提交，失败保留用户输入。 */
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
/** 刷新已生效的网站配置。 */
async function refresh() {
  snapshot.value = await api<Snapshot>("state");
}
/** 新建或编辑网站。 */
function edit(site?: Site) {
  error.value = "";
  siteTab.value = "settings";
  editing.value = site
    ? (JSON.parse(JSON.stringify(site)) as Site)
    : {
        id: "",
        revision: 0,
        directory: "",
        name: "",
        enabled: true,
        domains: [],
        listenIp: "0.0.0.0",
        ipv6Enabled: false,
        port: 80,
        kind: "static",
        runDir: ".",
        spa: false,
        upstream: "http://127.0.0.1:3000",
        certificateId: null,
        advanced: "",
      };
  codeEdited.value = false;
  domains.value = site?.domains.join("\n") ?? "";
}
/** 保存时同步执行配置检查和重载。 */
async function saveSite() {
  if (!editing.value) return;
  const site = JSON.parse(JSON.stringify(editing.value)) as Site;
  site.domains = domains.value.split(/[\s,]+/).filter(Boolean);
  if (!site.domains.length) throw new LocalizedError("主域名必填");
  site.name = site.name.trim() || site.domains[0]!;
  site.directory = site.directory.trim();
  site.listenIp = "0.0.0.0";
  site.port = Number(site.port);
  site.certificateId = site.certificateId || null;
  const saved = await api<Site>(
    site.id ? `sites/${site.id}` : "sites",
    site.id ? "PUT" : "POST",
    site,
  );
  editing.value = saved;
  domains.value = saved.domains.join("\n");
  notice.value = "已保存并重载";
  await refresh();
}
/** 删除直接生效，文件删除由独立选项决定。 */
async function removeSite() {
  if (!deleting.value) return;
  await api(
    `sites/${deleting.value.id}?deleteFiles=${deleteFiles.value}`,
    "DELETE",
  );
  deleting.value = null;
  await refresh();
  notice.value = "网站已删除";
}
/** 通过主控 Bridge 在套件所在节点打开文件管理。 */
async function openDirectory(site: Site | null = editing.value) {
  if (!site) return;
  if (
    !bridge.navigate({
      target: "app",
      value: "file-manager",
      payload: { path: directoryPath(site) },
    })
  )
    throw new LocalizedError(
      "请从 Veydeck 控制台打开网站管理，以使用文件管理应用",
    );
}
/** 切换工作区并读取对应事实。 */
function openMenu(value: string) {
  error.value = "";
  notice.value = "";
  if (value === "logs") {
    logKind.value = "error";
  }
}
/** 网站详情日志仅查询选中网站。 */
function openSiteTab(value: string) {
  if (value === "logs") {
    logKind.value = "access";
  }
}
/** 上传证书并清空私钥输入。 */
async function addCertificate() {
  await api("certificates", "POST", certificate.value);
  certificate.value = { certificate: "", key: "" };
  tab.value = "certs";
  await refresh();
}
/** 下载套件备份。 */
async function backup() {
  const response = await apiResponse("backup");
  const url = URL.createObjectURL(await response.blob()),
    link = document.createElement("a");
  link.href = url;
  link.download = "websites-backup.tar.gz";
  link.click();
  setTimeout(() => URL.revokeObjectURL(url), 1000);
}
onMounted(() => {
  void run(refresh);
  certificateClock = setInterval(() => {
    certificateNow.value = Date.now();
  }, 60000);
});
onBeforeUnmount(() => clearInterval(certificateClock));
onBeforeUnmount(unsubscribe);
</script>
<template>
  <VeyGlassProvider :glass="glass"
    ><div class="workspace" data-page="website-manager">
      <aside class="workspace-sidebar" data-ui="sidebar">
        <VeyMenu
          v-model="tab"
          :items="menuItems"
          data-ui="workspace-menu"
          @update:model-value="openMenu"
        />
      </aside>
      <main class="workspace-main" data-slot="main">
        <VeyAlert
          v-if="error"
          type="error"
          :description="errorMessage(error)"
          closable
          @close="error = ''"
        /><VeyAlert
          v-if="notice"
          type="success"
          :description="t(notice)"
          closable
          @close="notice = ''"
        />
        <div
          class="content"
          :class="{
            'content--list': ['sites', 'certs', 'authorities'].includes(tab),
          }"
          data-slot="content"
          :aria-busy="busy"
        >
          <template v-if="tab === 'sites'">
            <div class="section-toolbar">
              <span class="muted">{{
                t("{count} 个网站", { count: snapshot.sites.length })
              }}</span>
              <div class="actions">
                <VeyButton :disabled="busy" @click="run(refresh)">{{
                  t("刷新")
                }}</VeyButton
                ><VeyButton type="primary" :disabled="busy" @click="edit()">{{
                  t("新建网站")
                }}</VeyButton>
              </div>
            </div>
            <PaginatedTable
              :label="t('网站列表')"
              :data="snapshot.sites"
              row-key="id"
              :columns="[
                { prop: 'name', label: t('名称'), align: 'left' },
                { slot: 'kind', label: t('类型'), width: 110, align: 'center' },
                {
                  slot: 'protocol',
                  label: t('协议'),
                  width: 90,
                  align: 'center',
                },
                {
                  slot: 'status',
                  label: t('状态'),
                  width: 90,
                  align: 'center',
                },
                {
                  slot: 'directory',
                  label: t('网站目录'),
                  width: 100,
                  align: 'center',
                },
                {
                  slot: 'actions',
                  label: t('操作'),
                  width: 160,
                  align: 'center',
                },
              ]"
              data-ui="sites-table"
            >
              <template #kind="{ row }">{{
                row.kind === "static" ? t("静态网站") : t("反向代理")
              }}</template>
              <template #protocol="{ row }"
                ><span data-ui="site-protocol">{{
                  row.certificateId ? "HTTPS" : "HTTP"
                }}</span></template
              >
              <template #status="{ row }"
                ><VeySwitch
                  :model-value="row.enabled"
                  :disabled="busy"
                  :aria-label="
                    row.enabled
                      ? t('停用网站 {name}', { name: row.name })
                      : t('启用网站 {name}', { name: row.name })
                  "
                  data-ui="site-enabled"
                  @update:model-value="
                    (value) => run(() => toggleSite(row, value))
                  "
              /></template>
              <template #directory="{ row }"
                ><VeyButton
                  :title="directoryPath(row)"
                  :aria-label="t('打开 {name} 的网站目录', { name: row.name })"
                  data-ui="table-site-directory"
                  @click="run(() => openDirectory(row))"
                  ><VeyIcon name="folder" :size="20" /></VeyButton
              ></template>
              <template #actions="{ row }"
                ><div class="actions table-actions" data-ui="table-actions">
                  <VeyButton size="small" @click="edit(row)">{{
                    t("详情")
                  }}</VeyButton
                  ><VeyButton
                    size="small"
                    type="danger"
                    @click="
                      deleting = row;
                      deleteFiles = false;
                    "
                    >{{ t("删除") }}</VeyButton
                  >
                </div></template
              >
            </PaginatedTable>
          </template>
          <template v-else-if="tab === 'certs'">
            <div class="section-toolbar">
              <span class="muted">{{
                t("{count} 张证书", { count: snapshot.certificates.length })
              }}</span
              ><VeyButton :disabled="busy" @click="run(refresh)">{{
                t("刷新")
              }}</VeyButton>
            </div>
            <PaginatedTable
              :label="t('证书列表')"
              :data="snapshot.certificates"
              row-key="id"
              data-ui="certificates-table"
              :columns="[
                {
                  slot: 'domain',
                  label: t('主域名'),
                  minWidth: 180,
                  align: 'left',
                },
                {
                  slot: 'domains',
                  label: t('其他域名'),
                  minWidth: 180,
                  align: 'left',
                },
                {
                  slot: 'issuer',
                  label: t('颁发组织'),
                  minWidth: 160,
                  align: 'left',
                },
                {
                  slot: 'status',
                  label: t('状态'),
                  width: 110,
                  align: 'center',
                },
                {
                  slot: 'expiry',
                  label: t('过期时间'),
                  minWidth: 210,
                  align: 'left',
                },
                {
                  slot: 'actions',
                  label: t('操作'),
                  width: 160,
                  align: 'center',
                },
              ]"
            >
              <template #domain="{ row }"
                ><span class="log-value">{{
                  certificateDomain(row)
                }}</span></template
              >
              <template #domains="{ row }"
                ><span class="log-value">{{
                  row.domains
                    .filter((d: string) => d !== row.domain)
                    .join("、") || "—"
                }}</span></template
              >
              <template #issuer="{ row }">{{
                row.issuerOrganization || "—"
              }}</template>
              <template #status="{ row }"
                ><VeyTag :type="status(row).type">{{
                  t(status(row).label)
                }}</VeyTag></template
              >
              <template #expiry="{ row }">{{
                certificateTime(row.notAfter)
              }}</template>
              <template #actions="{ row }"
                ><div class="actions table-actions" data-ui="table-actions">
                  <VeyButton size="small" @click="certificateDetail = row">{{
                    t("详情")
                  }}</VeyButton
                  ><VeyButton
                    size="small"
                    type="danger"
                    @click="deletingCertificate = row"
                    >{{ t("删除") }}</VeyButton
                  >
                </div></template
              >
            </PaginatedTable>
          </template>
          <CertificateAuthorities
            v-else-if="tab === 'authorities'"
            @issued="run(issuedCertificate)"
          />
          <template v-else-if="tab === 'upload'">
            <div class="editor certificate-upload" data-ui="certificate-upload">
              <label
                >{{ t("证书 PEM")
                }}<textarea
                  v-model="certificate.certificate"
                  rows="8"
                  spellcheck="false"
                />
              </label>
              <label
                >{{ t("私钥 PEM")
                }}<textarea
                  v-model="certificate.key"
                  rows="8"
                  spellcheck="false"
                  autocomplete="off"
                />
              </label>
              <VeyAlert
                v-if="error"
                type="error"
                :description="errorMessage(error)"
              />
              <div class="actions">
                <VeyButton
                  type="primary"
                  :disabled="busy"
                  @click="run(addCertificate)"
                  >{{ t("上传证书") }}</VeyButton
                >
              </div>
            </div>
          </template>
          <PerformanceSettings v-else-if="tab === 'performance'" />
          <template v-else-if="tab === 'backup'">
            <div class="editor" data-ui="suite-backup">
              <h2 class="page-title">{{ t("备份") }}</h2>
              <p class="muted">
                {{
                  t(
                    "下载网站文件、配置、数据库及证书材料的完整备份，包含网站私钥和根证书私钥。",
                  )
                }}
              </p>
              <div class="actions">
                <VeyButton
                  type="primary"
                  :disabled="busy"
                  data-ui="download-backup"
                  @click="run(backup)"
                  >{{ t("下载备份") }}</VeyButton
                >
              </div>
            </div>
          </template>
          <template v-else-if="tab === 'logs'"
            ><div class="section-toolbar">
              <VeySelect
                :placeholder="t('请选择')"
                v-model="logKind"
                :options="[
                  { value: 'error', label: t('运行错误日志') },
                  { value: 'access', label: t('未匹配网站的访问日志') },
                ]"
                data-ui="engine-log-kind"
              /><VeyButton :disabled="logsLoading" @click="readLogs()">{{
                t("刷新")
              }}</VeyButton>
            </div>
            <LogViewer
              data-ui="engine-log"
              :kind="logKind"
              :lines="logLines"
              :loading="logsLoading"
              :error="logsError"
            />
          </template>
        </div>
      </main>
    </div>
    <VeyDialog
      :visible="!!editing"
      :title="
        editing?.id
          ? t('网站详情 · {name}', { name: editing.name })
          : t('新建网站')
      "
      width="900px"
      :close-on-click-overlay="!busy"
      @close="!busy && (editing = null)"
    >
      <div v-if="editing" class="editor" data-ui="site-editor">
        <VeyTabs
          v-if="editing.id"
          v-model="siteTab"
          :tabs="siteTabs"
          @update:model-value="openSiteTab"
        />
        <template v-if="siteTab === 'settings'">
          <div class="grid">
            <label
              >{{ t("名称")
              }}<VeyInput
                v-model="editing.name"
                :placeholder="t('默认使用主域名')" /></label
            ><label
              >{{ t("类型")
              }}<VeySelect
                :placeholder="t('请选择')"
                v-model="editing.kind"
                :options="[
                  { value: 'static', label: t('静态网站') },
                  { value: 'proxy', label: t('反向代理') },
                ]" /></label
            ><label
              >{{ t("代号")
              }}<VeyInput
                v-model="editing.directory"
                :disabled="!!editing.id"
                :placeholder="t('默认使用第一个域名，可自行修改')"
                data-ui="site-code"
                @update:model-value="codeEdited = true" /></label
            ><label
              >{{ t("端口") }}<VeyInput v-model="editing.port" type="number"
            /></label>
          </div>
          <label
            >{{ t("域名（必填，第一行为主域名）")
            }}<textarea
              v-model="domains"
              rows="3"
              placeholder="example.com&#10;www.example.com"
              data-ui="site-domains"
            />
          </label>
          <span class="muted">{{
            t(
              "同一端口内域名不能重复，不同端口可以复用域名或 IP；填写 0.0.0.0 可接收该端口的所有域名。代号在节点内唯一，创建后固定。",
            )
          }}</span>
          <VeyCheckbox v-model="editing.enabled">{{
            t("启用网站")
          }}</VeyCheckbox>
          <label
            >{{ t("监听 IPv6")
            }}<VeySwitch
              v-model="editing.ipv6Enabled"
              data-ui="site-ipv6-switch"
          /></label>
          <span class="muted">{{
            t("默认仅监听 IPv4；开启后同时监听 [::]:{port}。", {
              port: String(editing.port),
            })
          }}</span>
          <label v-if="editing.kind === 'proxy'"
            >{{ t("上游地址") }}<VeyInput v-model="editing.upstream"
          /></label>
          <VeyCheckbox v-if="editing.kind === 'static'" v-model="editing.spa">{{
            t("SPA 回退")
          }}</VeyCheckbox>
          <label
            >{{ t("证书")
            }}<VeySelect
              :placeholder="t('无证书（HTTP）')"
              v-model="editing.certificateId"
              data-ui="site-certificate"
              :options="certificates"
          /></label>
          <label
            >{{ t("高级 Nginx 配置")
            }}<textarea
              v-model="editing.advanced"
              rows="4"
              spellcheck="false"
            />
          </label>
        </template>
        <template v-else-if="siteTab === 'directory'"
          ><div data-ui="site-directory">
            <label>{{ t("网站目录") }}</label>
            <div class="actions path">
              <span class="mono">{{ sitePath }}</span
              ><VeyButton
                :aria-label="t('在文件管理中打开网站目录')"
                :title="t('在文件管理中打开网站目录')"
                data-ui="open-site-directory"
                @click="run(() => openDirectory())"
                ><VeyIcon name="folder" :size="20"
              /></VeyButton>
            </div>
          </div>
          <template v-if="editing.kind === 'static'"
            ><label
              >{{ t("运行目录（相对于网站目录）")
              }}<VeyInput
                v-model="editing.runDir"
                :placeholder="t('. 或 dist')"
                data-ui="run-directory" /></label
            ><span class="mono muted">{{
              t("实际运行路径：{path}", {
                path:
                  sitePath +
                  (editing.runDir === "." ? "" : `/${editing.runDir}`),
              })
            }}</span>
            <p class="muted">
              {{
                t(
                  "在文件管理中上传文件或目录。当前运行目录中的文件修改直接生效；修改运行目录后点击「保存并重载」。",
                )
              }}
            </p></template
          >
        </template>
        <template v-else-if="siteTab === 'logs'"
          ><div class="section-toolbar">
            <VeySelect
              :placeholder="t('请选择')"
              v-model="logKind"
              :options="[
                { value: 'access', label: t('访问日志') },
                { value: 'error', label: t('错误日志') },
              ]"
              data-ui="site-log-kind"
            /><VeyButton :disabled="logsLoading" @click="readLogs()">{{
              t("刷新")
            }}</VeyButton>
          </div>
          <LogViewer
            data-ui="site-log"
            :kind="logKind"
            :lines="logLines"
            :loading="logsLoading"
            :error="logsError"
          />
        </template>
        <VeyAlert
          v-if="error"
          type="error"
          :description="errorMessage(error)"
        /><VeyAlert v-if="notice" type="success" :description="t(notice)" />
      </div>
      <template #footer
        ><VeyButton :disabled="busy" @click="editing = null">{{
          t("关闭")
        }}</VeyButton
        ><VeyButton
          v-if="siteTab !== 'logs'"
          type="primary"
          :disabled="busy"
          data-ui="save-site"
          @click="run(saveSite)"
          >{{ editing?.id ? t("保存并重载") : t("创建网站") }}</VeyButton
        ></template
      >
    </VeyDialog>
    <VeyDialog
      :visible="!!deleting"
      :title="t('删除网站')"
      width="480px"
      @close="!busy && (deleting = null)"
      ><div class="editor">
        <p>
          {{ t("删除 {name} 后立即停止该网站。", { name: deleting?.name }) }}
        </p>
        <VeyCheckbox v-model="deleteFiles">{{
          t("同时永久删除网站目录及文件")
        }}</VeyCheckbox
        ><VeyAlert
          v-if="error"
          type="error"
          :description="errorMessage(error)"
        />
      </div>
      <template #footer
        ><VeyButton :disabled="busy" @click="deleting = null">{{
          t("取消")
        }}</VeyButton
        ><VeyButton type="danger" :disabled="busy" @click="run(removeSite)">{{
          t("确认删除")
        }}</VeyButton></template
      ></VeyDialog
    >
    <VeyDialog
      :visible="!!certificateDetail"
      :title="t('证书详情')"
      width="800px"
      @close="certificateDetail = null"
    >
      <div data-ui="certificate-detail" class="log-value">
        <VeyDescriptions :items="certificateDetails" :column="1" />
      </div>
      <template #footer
        ><VeyButton @click="certificateDetail = null">{{
          t("关闭")
        }}</VeyButton></template
      >
    </VeyDialog>
    <VeyDialog
      :visible="!!deletingCertificate"
      :title="t('删除证书')"
      width="480px"
      @close="deletingCertificate = null"
      ><p>
        {{
          t("确认删除 {name}？仍被网站引用的证书不能删除。", {
            name: deletingCertificate
              ? certificateDomain(deletingCertificate)
              : "",
          })
        }}
      </p>
      <VeyAlert
        v-if="error"
        type="error"
        :description="errorMessage(error)"
      /><template #footer
        ><VeyButton :disabled="busy" @click="deletingCertificate = null">{{
          t("取消")
        }}</VeyButton
        ><VeyButton
          type="danger"
          :disabled="busy"
          @click="
            run(async () => {
              await api(`certificates/${deletingCertificate?.id}`, 'DELETE');
              deletingCertificate = null;
              await refresh();
            })
          "
          >{{ t("确认删除") }}</VeyButton
        ></template
      ></VeyDialog
    >
  </VeyGlassProvider>
</template>

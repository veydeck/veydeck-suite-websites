/** 通过公开 Bridge 同步主题、语言和通知。 */
import { createSuiteBridge } from "@veydeck/suite-sdk";
import { watch } from "vue";
import { locale, t } from "./i18n";
export { locale, t } from "./i18n";
export const bridge = createSuiteBridge({
  capabilities: ["theme", "locale", "notification", "window", "navigation"],
  supportedLocales: ["zh-CN", "en-US"],
  defaultLocale: "zh-CN",
});
locale.value =
  bridge.getLocaleSnapshot().locale === "en-US" ? "en-US" : "zh-CN";
bridge.subscribeLocale((v) => {
  locale.value = v.locale === "en-US" ? "en-US" : "zh-CN";
});
bridge.ready();

/** 同步文档及宿主窗口标题，不重新挂载业务页面。 */
watch(
  locale,
  (value) => {
    document.documentElement.lang = value;
    document.title = t("网站管理");
    bridge.postMessage("suite:window:title:update", { title: t("网站管理") });
  },
  { immediate: true },
);

/** 网站套件 Vue 入口。 */
import { createApp } from "vue";
import "@veydeck/tokens";
import "@veydeck/vue/style.css";
import "./style.css";
import App from "./App.vue";
createApp(App).mount("#app");

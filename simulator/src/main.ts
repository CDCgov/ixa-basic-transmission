import { createApp } from "vue";
import { createRouter, createWebHistory } from "vue-router";
import "cfasim-ui/theme/all";
import App from "./App.vue";
import Page from "./Page.vue";
import CalibratePage from "./CalibratePage.vue";

// Two routes: the main simulator and the ABC-SMC calibration page.
// Both use `useRouter`/`useRoute` via `cfasim-ui/shared`'s `useUrlParams`
// to round-trip params through the URL query string.
const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [
    { path: "/calibrate", component: CalibratePage },
    // The simulate route carries the main-area tab in its path: "/" (or
    // "/simulate") = run charts, "/explore" = the rate-function explainer.
    // One route record + optional param → switching tabs reuses `Page.vue`
    // (no remount), so the model params and in-flight simulation persist.
    { path: "/:view(simulate|explore)?", component: Page },
  ],
});

createApp(App).use(router).mount("#app");

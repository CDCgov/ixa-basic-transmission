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
    { path: "/", component: Page },
    { path: "/calibrate", component: CalibratePage },
  ],
});

createApp(App).use(router).mount("#app");

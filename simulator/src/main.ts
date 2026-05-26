import { createApp } from "vue";
import { createRouter, createWebHistory } from "vue-router";
import "cfasim-ui/theme/all";
import App from "./App.vue";
import Page from "./Page.vue";

// Single-page router. Page.vue uses `useRouter`/`useRoute` via
// `cfasim-ui/shared`'s `useUrlParams` to round-trip params through the
// URL query string — the router instance has to exist for those calls
// to resolve, even though there's only one route.
const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [{ path: "/", component: Page }],
});

createApp(App).use(router).mount("#app");

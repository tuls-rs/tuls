import type { Theme } from "vitepress";
import DefaultTheme from "vitepress/theme";
import ServerCard from "./components/ServerCard.vue";
import "./style.css";

export default {
  extends: DefaultTheme,
  enhanceApp({ app }) {
    app.component("ServerCard", ServerCard);
  },
} satisfies Theme;

import { boot } from "./core/app.js";
import { initLanguage } from "./core/i18n.js";
import { initTheme } from "./core/theme.js";
import "./features/index.js";

initTheme();
initLanguage();
boot();

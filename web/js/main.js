import { boot } from "./core/app.js";
import { initLanguage } from "./core/i18n.js";
import "./features/index.js";

initLanguage();
boot();

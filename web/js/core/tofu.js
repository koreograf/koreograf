import { openDialog } from "./dialog.js";
import { h } from "./dom.js";
import { t } from "./i18n.js";

export function confirmTofuApply(item) {
  return new Promise((resolve) => {
    let settled = false;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      resolve(value);
    };
    openDialog({
      id: "tofu-apply-confirm",
      title: t("opentofu_apply_confirmation_title"),
      body: `<p>${h("opentofu_apply_confirmation_body", { name: item.name })}</p>`,
      submit: t("apply"),
      onSubmit: ({ close }) => {
        finish(true);
        close();
      },
      onClose: () => finish(false),
    });
  });
}

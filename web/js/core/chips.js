import { esc, h } from "./dom.js";
import { t } from "./i18n.js";

export function parseTagList(value) {
  return String(value || "").split(",").map((part) => part.trim()).filter(Boolean);
}

export function validTag(part) {
  return part.length > 0
    && part.length <= 80
    && !part.startsWith("-")
    && /^[A-Za-z0-9_.:-]+$/.test(part);
}

export function splitDraft(raw, flush) {
  const parts = String(raw || "").split(/[,\s]+/);
  const trailing = /[,\s]$/.test(raw);
  if (flush || trailing) return { tokens: parts.filter(Boolean), rest: "" };
  return { tokens: parts.slice(0, -1).filter(Boolean), rest: parts.at(-1) || "" };
}

export function readChips(box) {
  return [...box.querySelectorAll(".chip-text")].map((node) => node.textContent);
}

export function renderChips(box, tokens) {
  const entry = box.querySelector(".chip-entry");
  box.querySelectorAll(".chip").forEach((node) => node.remove());
  const html = tokens.map((token) => `<span class="chip"><span class="chip-text">${esc(token)}</span><button class="chip-remove" type="button" aria-label="${h("remove_tag_name", { name: token })}">×</button></span>`).join("");
  entry.insertAdjacentHTML("beforebegin", html);
  entry.placeholder = tokens.length ? "" : t("enter_a_tag");
  const hidden = box.parentElement.querySelector(`input[type="hidden"][name="${CSS.escape(box.dataset.chips)}"]`);
  if (hidden) hidden.value = tokens.join(",");
  box.querySelectorAll(".chip-remove").forEach((button) => {
    button.addEventListener("mousedown", (event) => event.preventDefault());
    button.addEventListener("click", () => {
      const name = button.parentElement.querySelector(".chip-text").textContent;
      renderChips(box, readChips(box).filter((item) => item !== name));
      const error = box.parentElement.querySelector(".chip-error");
      if (error) error.textContent = "";
      entry.focus();
    });
  });
}

export function addTokens(box, tokens) {
  const error = box.parentElement.querySelector(".chip-error");
  const chips = readChips(box);
  for (let index = 0; index < tokens.length; index += 1) {
    const token = tokens[index];
    if (chips.includes(token)) continue;
    const tooLong = [...chips, token].join(",").length > 300;
    if (!validTag(token) || tooLong) {
      if (error) {
        error.textContent = tooLong
          ? t("the_tags_are_too_long")
          : t("the_tag_part_is_invalid_letters_digits_and", { part: token });
      }
      renderChips(box, chips);
      return { ok: false, keep: tokens.slice(index) };
    }
    chips.push(token);
  }
  if (error) error.textContent = "";
  renderChips(box, chips);
  return { ok: true, keep: [] };
}

export function takeDraft(box, flush) {
  const entry = box.querySelector(".chip-entry");
  const { tokens, rest } = splitDraft(entry.value, flush);
  if (!tokens.length) {
    entry.value = rest;
    return true;
  }
  const result = addTokens(box, tokens);
  const leftover = result.ok ? rest : [...result.keep, rest].filter(Boolean).join(" ");
  entry.value = leftover;
  return result.ok;
}

export function bindChips(form) {
  form.querySelectorAll("[data-chips]").forEach((box) => {
    const hidden = box.parentElement.querySelector(`input[type="hidden"][name="${CSS.escape(box.dataset.chips)}"]`);
    renderChips(box, parseTagList(hidden ? hidden.value : ""));
    const entry = box.querySelector(".chip-entry");
    const error = box.parentElement.querySelector(".chip-error");
    entry.addEventListener("keydown", (event) => {
      if (event.key === "Enter" || event.key === "," || event.key === " ") {
        event.preventDefault();
        takeDraft(box, true);
      } else if (event.key === "Backspace" && entry.value === "") {
        const tokens = readChips(box);
        if (!tokens.length) return;
        event.preventDefault();
        renderChips(box, tokens.slice(0, -1));
        if (error) error.textContent = "";
      }
    });
    entry.addEventListener("input", () => {
      if (error) error.textContent = "";
      if (/[,\s]/.test(entry.value)) takeDraft(box, false);
    });
    entry.addEventListener("blur", () => takeDraft(box, true));
    box.addEventListener("click", (event) => {
      if (event.target === box) entry.focus();
    });
  });
}

export function flushChips(form) {
  let ok = true;
  form.querySelectorAll("[data-chips]").forEach((box) => {
    if (box.closest("[hidden]")) return;
    if (!takeDraft(box, true)) ok = false;
  });
  return ok;
}

export function chipBox(name, label, value) {
  return `<label class="field chips-field"><span>${h(label)}</span>
    <div class="chips" data-chips="${esc(name)}">
      <input class="chip-entry" type="text" autocomplete="off" aria-label="${h(label)}">
    </div>
    <input type="hidden" name="${esc(name)}" value="${esc(parseTagList(value).join(","))}">
    <span class="chip-error" aria-live="polite"></span>
  </label>`;
}

export function chipReadout(value) {
  const tokens = parseTagList(value);
  if (!tokens.length) return "";
  return `<span class="chips readonly">${tokens.map((token) => `<span class="chip"><span class="chip-text">${esc(token)}</span></span>`).join("")}</span>`;
}

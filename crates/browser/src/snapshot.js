// Runs in the page: numbers the visible things a person could act on (as `data-tk-ref`) and
// returns them with the page's text. After super-agent's, which this is a port of.
(() => {
  try {
    const MAX = 150;
    for (const el of document.querySelectorAll("[data-tk-ref]")) el.removeAttribute("data-tk-ref");
    const selector =
      'a[href], button, input:not([type=hidden]), textarea, select, summary, [role=button], [role=link], [role=checkbox], [role=radio], [role=tab], [role=menuitem], [role=option], [role=switch], [role=combobox], [role=textbox], [role=searchbox], [contenteditable=""], [contenteditable=true]';
    const clean = (t) => (t || "").replace(/\s+/g, " ").trim();
    const visible = (el) => {
      const r = el.getBoundingClientRect();
      if (r.width < 2 || r.height < 2) return false;
      const s = getComputedStyle(el);
      return s.visibility !== "hidden" && s.display !== "none" && s.opacity !== "0";
    };
    const roleOf = (el, tag, type) => {
      const role = el.getAttribute("role");
      if (role) return role;
      if (tag === "a") return "link";
      if (tag === "select") return "select";
      if (tag === "textarea" || el.isContentEditable) return "textbox";
      if (tag === "input") {
        if (type === "checkbox" || type === "radio") return type;
        return ["submit", "button", "reset", "image"].includes(type) ? "button" : "textbox";
      }
      return "button";
    };
    const nameOf = (el, tag, type) => {
      const byIds = (ids) => clean(ids.split(" ").map((id) => document.getElementById(id)?.textContent).join(" "));
      const label = el.id ? document.querySelector('label[for="' + CSS.escape(el.id) + '"]') : el.closest("label");
      return clean(
        el.getAttribute("aria-label") ||
          (el.getAttribute("aria-labelledby") && byIds(el.getAttribute("aria-labelledby"))) ||
          (tag === "input" && ["submit", "button"].includes(type) ? el.value : "") ||
          (tag !== "input" && tag !== "textarea" && tag !== "select" && !el.isContentEditable ? el.innerText : "") ||
          (label && label.textContent) ||
          el.getAttribute("placeholder") ||
          el.getAttribute("title") ||
          el.getAttribute("alt") ||
          el.querySelector("img")?.getAttribute("alt") ||
          el.getAttribute("name") ||
          "",
      ).slice(0, 100);
    };
    const elements = [];
    for (const el of document.querySelectorAll(selector)) {
      if (elements.length >= MAX) break;
      if (!visible(el)) continue;
      const tag = el.tagName.toLowerCase();
      const type = (el.getAttribute("type") || "").toLowerCase();
      const role = roleOf(el, tag, type);
      const name = nameOf(el, tag, type);
      const ref = elements.length + 1;
      el.setAttribute("data-tk-ref", String(ref));
      const item = { ref, role, name };
      const autocomplete = (el.getAttribute("autocomplete") || "").toLowerCase();
      const secret = type === "password" || autocomplete.startsWith("cc-") || /card number|cvv|cvc|security code|expiration/i.test(name);
      if (role === "textbox" && secret) item.sensitive = true;
      // What is typed in a sensitive field never leaves the page.
      const field = role === "textbox" || role === "select" || role === "combobox" || role === "searchbox";
      if (field && el.value && !item.sensitive) item.value = String(el.value).slice(0, 100);
      // An editor holds its text as its content, not as a value: shown too, so what was
      // typed into a post box can be seen without typing it again.
      else if (role === "textbox" && el.isContentEditable && clean(el.innerText)) item.value = clean(el.innerText).slice(0, 300);
      if (tag === "a" && el.href) item.href = el.href;
      if (el.disabled || el.getAttribute("aria-disabled") === "true") item.disabled = true;
      elements.push(item);
    }
    const text = clean(document.body ? document.body.innerText : "").slice(0, 6000);
    return JSON.stringify({ url: location.href, title: document.title, text, elements });
  } catch (error) {
    return JSON.stringify({ error: String(error && error.message ? error.message : error) });
  }
})()

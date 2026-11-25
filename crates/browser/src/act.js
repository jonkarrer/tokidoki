// Runs in the page: acts on the element numbered `REF` by the last snapshot, as a person
// would. `ACTION` is "click" or "type"; `TEXT` and `SUBMIT` go with "type". The three are
// filled in by the caller as JSON.
(() => {
  try {
    const el = document.querySelector('[data-tk-ref="' + REF + '"]');
    if (!el) return JSON.stringify({ gone: true });
    el.scrollIntoView({ block: "center", inline: "center" });
    const r = el.getBoundingClientRect();
    const at = { bubbles: true, cancelable: true, composed: true, view: window, clientX: r.left + r.width / 2, clientY: r.top + r.height / 2, button: 0 };
    const fire = (type, Kind = MouseEvent, more = {}) => el.dispatchEvent(new Kind(type, { ...at, ...more }));

    if (ACTION === "click") {
      // A link that would open another window opens here: there is one page.
      if (el.tagName === "A" && el.target && el.target !== "_self") el.target = "_self";
      fire("pointerover", PointerEvent);
      fire("mouseover");
      fire("pointerdown", PointerEvent, { buttons: 1 });
      fire("mousedown", MouseEvent, { buttons: 1 });
      if (typeof el.focus === "function") el.focus();
      fire("pointerup", PointerEvent);
      fire("mouseup");
      // `click()` is what follows a link, checks a box and submits a form.
      el.click();
      return JSON.stringify({ ok: true });
    }

    if (typeof el.focus === "function") el.focus();
    const tag = el.tagName;
    if (tag === "SELECT") {
      const want = TEXT.trim().toLowerCase();
      const option = [...el.options].find((o) => o.value.toLowerCase() === want || o.text.trim().toLowerCase() === want);
      if (!option) return JSON.stringify({ error: "no option called " + TEXT + "; the options are: " + [...el.options].map((o) => o.text.trim()).join(", ") });
      el.value = option.value;
      el.dispatchEvent(new Event("input", { bubbles: true }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
    } else if (tag === "INPUT" || tag === "TEXTAREA") {
      // Through the element's own setter, so a framework that wraps `value` (React) sees it.
      const proto = tag === "INPUT" ? HTMLInputElement.prototype : HTMLTextAreaElement.prototype;
      Object.getOwnPropertyDescriptor(proto, "value").set.call(el, TEXT);
      el.dispatchEvent(new InputEvent("input", { bubbles: true, inputType: "insertText", data: TEXT }));
      el.dispatchEvent(new Event("change", { bubbles: true }));
    } else if (el.isContentEditable) {
      const selection = getSelection();
      selection.selectAllChildren(el);
      // Deprecated, and still the one way an editor (a post box, a comment box) takes text
      // as if it were typed.
      if (!document.execCommand("insertText", false, TEXT)) el.textContent = TEXT;
    } else {
      return JSON.stringify({ error: "that isn't something to type into" });
    }
    if (SUBMIT) {
      const key = { bubbles: true, cancelable: true, composed: true, key: "Enter", code: "Enter", keyCode: 13, which: 13 };
      const went = el.dispatchEvent(new KeyboardEvent("keydown", key));
      el.dispatchEvent(new KeyboardEvent("keypress", key));
      el.dispatchEvent(new KeyboardEvent("keyup", key));
      // A made-up Enter doesn't submit a form by itself: do what the page didn't stop.
      if (went && el.form && tag !== "TEXTAREA") {
        if (typeof el.form.requestSubmit === "function") el.form.requestSubmit();
        else el.form.submit();
      }
    }
    return JSON.stringify({ ok: true });
  } catch (error) {
    return JSON.stringify({ error: String(error && error.message ? error.message : error) });
  }
})()

// Runs in the page: whether it has finished loading and gone quiet. A watcher, set once per
// document, notes when the page last changed; "quiet" is QUIET ms without a change.
(() => {
  try {
    if (!window.__tkWatch) {
      window.__tkWatch = { changed: Date.now() };
      new MutationObserver(() => (window.__tkWatch.changed = Date.now())).observe(document.documentElement, {
        subtree: true,
        childList: true,
        attributes: true,
        characterData: true,
      });
    }
    return JSON.stringify({ ready: document.readyState === "complete", quiet: Date.now() - window.__tkWatch.changed >= QUIET, url: location.href });
  } catch (error) {
    return JSON.stringify({ ready: false, quiet: false, error: String(error) });
  }
})()

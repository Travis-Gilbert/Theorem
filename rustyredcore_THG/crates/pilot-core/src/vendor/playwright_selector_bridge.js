/*
 * Theorem selector bridge.
 *
 * Provenance: compatibility surface for Microsoft Playwright injected selectors,
 * checked against microsoft/playwright v1.61.0
 * (tag 1cc5a90cfa3eaa430b1a991963100f95126caa47).
 *
 * Playwright is Apache-2.0 licensed in the current package metadata. This file
 * is Theorem glue, not a verbatim copy of Playwright's injected engine. It gives
 * the Servo embedder a small stable wrapper until the full generated injected
 * bundle is vendored.
 */
(function () {
  function textOf(element) {
    return [
      element.getAttribute("aria-label"),
      element.getAttribute("title"),
      element.getAttribute("placeholder"),
      element.getAttribute("alt"),
      element.textContent
    ].filter(Boolean).join(" ").trim();
  }

  function roleOf(element) {
    const explicit = element.getAttribute("role");
    if (explicit) return explicit.toLowerCase();
    const tag = element.tagName.toLowerCase();
    if (tag === "a" && element.hasAttribute("href")) return "link";
    if (tag === "button") return "button";
    if (tag === "textarea") return "textbox";
    if (tag === "select") return "select";
    if (tag === "input") return (element.getAttribute("type") || "text").toLowerCase();
    return tag;
  }

  window.theoremQuerySelectorAll = function theoremQuerySelectorAll(selector) {
    if (selector.startsWith("role=")) {
      const role = selector.slice("role=".length).toLowerCase();
      return Array.from(document.querySelectorAll("a[href],button,input,select,textarea,[role]"))
        .filter(element => roleOf(element) === role);
    }
    if (selector.startsWith("text=")) {
      const text = selector.slice("text=".length).toLowerCase();
      return Array.from(document.querySelectorAll("a[href],button,input,select,textarea,[role]"))
        .filter(element => textOf(element).toLowerCase().includes(text));
    }
    if (selector.startsWith("testid=")) {
      const id = selector.slice("testid=".length);
      return Array.from(document.querySelectorAll(`[data-testid="${CSS.escape(id)}"],[data-test-id="${CSS.escape(id)}"],[data-test="${CSS.escape(id)}"]`));
    }
    return Array.from(document.querySelectorAll(selector));
  };
})();

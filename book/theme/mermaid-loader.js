/* Mermaid diagram renderer for quack-rs mdBook documentation.
 * Converts ```mermaid code blocks into SVG diagrams. Mermaid itself (~1 MB)
 * is fetched from the CDN only on pages that contain a diagram, after the
 * page has loaded, and diagrams are redrawn when the reader switches theme.
 */
(function () {
  var DARK = ["navy", "coal", "ayu"];

  function isDark() {
    var cl = document.documentElement.classList;
    return DARK.some(function (t) { return cl.contains(t); });
  }

  function start() {
    var blocks = document.querySelectorAll("code.language-mermaid");
    if (!blocks.length) return;

    // Swap each <pre> for a container that keeps the diagram source.
    var targets = [];
    for (var i = 0; i < blocks.length; i++) {
      var pre = blocks[i].parentElement;
      var div = document.createElement("div");
      div.className = "mermaid-diagram";
      div.setAttribute("data-source", blocks[i].textContent);
      div.appendChild(pre.cloneNode(true)); // shown until Mermaid renders
      pre.parentNode.replaceChild(div, pre);
      targets.push(div);
    }

    import("https://cdn.jsdelivr.net/npm/mermaid@11.17.2/dist/mermaid.esm.min.mjs").then(function (m) {
      var mermaid = m.default;
      var seq = 0;
      var dark = null;

      function renderAll() {
        if (dark === isDark()) return;
        dark = isDark();
        mermaid.initialize({ startOnLoad: false, theme: dark ? "dark" : "default" });
        targets.forEach(function (div) {
          mermaid.render("mermaid-" + (++seq), div.getAttribute("data-source")).then(
            function (result) { div.innerHTML = result.svg; },
            function (err) { console.error("mermaid:", err); }
          );
        });
      }

      renderAll();
      new MutationObserver(renderAll).observe(document.documentElement, {
        attributes: true,
        attributeFilter: ["class"],
      });
    });
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start);
  } else {
    start();
  }
})();

/* Mermaid diagram renderer for quack-rs mdBook documentation.
 * Loads Mermaid from CDN and converts ```mermaid code blocks into SVG diagrams.
 */
(function () {
  function renderMermaid() {
    var blocks = document.querySelectorAll("code.language-mermaid");
    if (!blocks.length) return;

    var script = document.createElement("script");
    script.type = "module";
    script.textContent = [
      'import mermaid from "https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs";',
      'var isDark = document.documentElement.classList.contains("navy") ||',
      '             document.documentElement.classList.contains("coal") ||',
      '             document.documentElement.classList.contains("ayu");',
      'mermaid.initialize({ startOnLoad: false, theme: isDark ? "dark" : "default" });',
      'var blocks = document.querySelectorAll("code.language-mermaid");',
      'var idx = 0;',
      'for (var i = 0; i < blocks.length; i++) {',
      '  (function(block) {',
      '    var id = "mermaid-" + (++idx);',
      '    var source = block.textContent;',
      '    mermaid.render(id, source).then(function(result) {',
      '      var div = document.createElement("div");',
      '      div.className = "mermaid-diagram";',
      '      div.innerHTML = result.svg;',
      '      var pre = block.parentElement;',
      '      pre.parentNode.replaceChild(div, pre);',
      '    });',
      '  })(blocks[i]);',
      '}',
    ].join("\n");
    document.head.appendChild(script);
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", renderMermaid);
  } else {
    renderMermaid();
  }
})();

/* SPDX-License-Identifier: MIT
 * Copyright 2026 Tom F.
 *
 * Version switcher for the versioned quack-rs book (quack-rs.com).
 *
 * scripts/build-versioned-site.py writes this file to the site root with the
 * list of published versions substituted for the VERSIONS placeholder below,
 * and links it from every page of every version. It adds a version menu to
 * the mdBook menu bar and, on a page that is not the latest release, a banner
 * that says so and links to the same page in the latest release.
 *
 * Site layout (see the builder's docstring):
 *   /            the latest release
 *   /vX.Y/       each release line (the newest patch of it)
 *   /dev/        the unreleased main branch
 */
(function () {
  "use strict";

  var DATA = /*@@VERSIONS@@*/ null;
  if (!DATA || !DATA.versions || !DATA.versions.length) return;

  // The site root is the directory this script was served from, so the
  // switcher works on quack-rs.com, on a fork's Pages site and from a local
  // `python3 -m http.server` preview alike.
  var script = document.currentScript;
  if (!script) return;
  var siteRoot = script.src.replace(/[^/]*$/, "");
  if (window.location.href.indexOf(siteRoot) !== 0) return;

  var bySlug = {};
  for (var i = 0; i < DATA.versions.length; i++) {
    bySlug[DATA.versions[i].slug] = DATA.versions[i];
  }

  // Work out which version this page belongs to and its path within it.
  var rest = window.location.href.slice(siteRoot.length).split(/[?#]/)[0];
  var first = rest.split("/")[0];
  var current, page;
  if (bySlug[first]) {
    current = bySlug[first];
    page = rest.slice(first.length + 1);
  } else {
    current = bySlug[DATA.latest];
    page = rest;
  }
  if (!current) return;
  if (page === "" || page === "index.html") page = "introduction.html";

  function urlFor(version, pagePath) {
    var base = version.slug === DATA.latest ? siteRoot : siteRoot + version.slug + "/";
    if (version.pages.indexOf(pagePath) === -1) return base;
    return pagePath === "introduction.html" ? base : base + pagePath;
  }

  function menu() {
    var select = document.createElement("select");
    select.className = "qrs-version-select";
    select.setAttribute("aria-label", "Documentation version");
    select.title = "Documentation version";
    for (var j = 0; j < DATA.versions.length; j++) {
      var v = DATA.versions[j];
      var opt = document.createElement("option");
      opt.value = v.slug;
      opt.textContent = v.label;
      if (v.slug === current.slug) opt.selected = true;
      select.appendChild(opt);
    }
    select.addEventListener("change", function () {
      var target = bySlug[select.value];
      if (target) window.location.href = urlFor(target, page);
    });
    var holder = document.querySelector("#menu-bar .left-buttons");
    if (holder) holder.appendChild(select);
  }

  function banner() {
    var latest = bySlug[DATA.latest];
    if (current.slug === DATA.latest || !latest) return;
    var div = document.createElement("div");
    div.className = "qrs-version-banner";
    div.setAttribute("role", "note");
    // "quack-rs" must not break at its hyphen on a narrow screen.
    var name = document.createElement("span");
    name.className = "qrs-nowrap";
    name.textContent = "quack-rs";
    if (current.slug === "dev") {
      div.appendChild(document.createTextNode("This is the documentation for the unreleased development version of "));
      div.appendChild(name);
      div.appendChild(document.createTextNode(". "));
    } else {
      div.appendChild(document.createTextNode("This is the documentation for "));
      div.appendChild(name);
      div.appendChild(document.createTextNode(" " + current.version + ", an older release. "));
    }
    var a = document.createElement("a");
    a.href = urlFor(latest, page);
    a.textContent = "Read it for the latest release, " + latest.version + ".";
    div.appendChild(a);
    var main = document.querySelector("#content main") || document.querySelector("main");
    if (main) main.insertBefore(div, main.firstChild);
  }

  function start() {
    menu();
    banner();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", start);
  } else {
    start();
  }
})();

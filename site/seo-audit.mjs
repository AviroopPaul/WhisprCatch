#!/usr/bin/env node
// SEO audit for the WhisprCatch site.
//
//   node site/seo-audit.mjs                    # audit local files
//   node site/seo-audit.mjs --live             # audit the deployed site
//
// Exits non-zero on an error, so CI can gate a site PR on it. The point is the
// mechanical regressions that are invisible in review: a title that grew past
// the truncation limit, a page missing from the sitemap, JSON-LD that stopped
// parsing. Strategy is not testable; these are.

import { readFile, readdir, stat } from "node:fs/promises";
import { join, dirname, relative } from "node:path";
import { fileURLToPath } from "node:url";

const SITE = dirname(fileURLToPath(import.meta.url));
const ORIGIN = "https://whisper-catch.vercel.app";
const LIVE = process.argv.includes("--live");

// Google renders titles to roughly 580px and descriptions to roughly 920px.
// Character counts are a proxy, so these are advisory bounds, not exact.
const TITLE_MAX = 60;
const TITLE_MIN = 15;
const DESC_MAX = 160;
const DESC_MIN = 70;
// Anything larger competes with the LCP element for bandwidth on a cold visit.
const ASSET_WARN_KB = 500;

let errors = 0;
let warnings = 0;
const fail = (page, msg) => {
  errors++;
  console.log(`  \x1b[31mFAIL\x1b[0m ${page}: ${msg}`);
};
const warn = (page, msg) => {
  warnings++;
  console.log(`  \x1b[33mWARN\x1b[0m ${page}: ${msg}`);
};
const pass = (page, msg) => console.log(`  \x1b[32mok\x1b[0m   ${page}: ${msg}`);

const decode = (s) =>
  s
    ? s
        .replace(/&amp;/g, "&")
        .replace(/&lt;/g, "<")
        .replace(/&gt;/g, ">")
        .replace(/&quot;/g, '"')
        .replace(/&#39;/g, "'")
        .trim()
    : s;

const grab = (html, re) => {
  const m = html.match(re);
  return m ? decode(m[1]) : null;
};

async function htmlPages() {
  const found = [];
  async function walk(dir) {
    for (const entry of await readdir(dir)) {
      if (entry === "node_modules" || entry.startsWith(".")) continue;
      const full = join(dir, entry);
      if ((await stat(full)).isDirectory()) await walk(full);
      else if (entry.endsWith(".html")) found.push(full);
    }
  }
  await walk(SITE);
  // the Search Console verification stub is not a page
  return found.filter((f) => !/google[0-9a-f]+\.html$/.test(f));
}

/** Local file path -> the URL path it is served at. */
function urlPathFor(file) {
  const rel = relative(SITE, file).replace(/\\/g, "/");
  return "/" + rel.replace(/(^|\/)index\.html$/, "$1");
}

async function loadPage(file) {
  const path = urlPathFor(file);
  if (!LIVE) return { path, html: await readFile(file, "utf8") };
  const res = await fetch(ORIGIN + path, { redirect: "follow" });
  if (!res.ok) throw new Error(`${res.status} fetching ${path}`);
  return { path, html: await res.text() };
}

function auditPage(path, html) {
  const title = grab(html, /<title>(.*?)<\/title>/is);
  if (!title) fail(path, "no <title>");
  else if (title.length > TITLE_MAX)
    fail(path, `title is ${title.length} chars, truncated past ~${TITLE_MAX}: "${title}"`);
  else if (title.length < TITLE_MIN) warn(path, `title is only ${title.length} chars`);
  else pass(path, `title ${title.length} chars`);

  const desc = grab(html, /<meta\s+name="description"\s+content="(.*?)"/is);
  if (!desc) fail(path, "no meta description");
  else if (desc.length > DESC_MAX)
    fail(path, `description is ${desc.length} chars, truncated past ~${DESC_MAX}`);
  else if (desc.length < DESC_MIN) warn(path, `description is only ${desc.length} chars`);
  else pass(path, `description ${desc.length} chars`);

  const canonical = grab(html, /<link\s+rel="canonical"\s+href="(.*?)"/is);
  if (!canonical) fail(path, "no canonical link");
  else if (canonical !== ORIGIN + path)
    fail(path, `canonical is ${canonical}, expected ${ORIGIN + path}`);
  else pass(path, "canonical matches");

  const h1s = html.match(/<h1[\s>]/gi) || [];
  if (h1s.length !== 1) fail(path, `${h1s.length} <h1> elements, expected exactly 1`);
  else pass(path, "one h1");

  // Open Graph / Twitter, so shared links render properly
  for (const prop of ["og:title", "og:description", "og:image"]) {
    const re = new RegExp(`<meta\\s+property="${prop}"\\s+content="(.*?)"`, "is");
    if (!grab(html, re)) warn(path, `missing ${prop}`);
  }

  // Structured data must parse — a stray comma silently kills rich results
  const blocks = [...html.matchAll(/<script[^>]*application\/ld\+json[^>]*>(.*?)<\/script>/gis)];
  if (blocks.length === 0) warn(path, "no JSON-LD");
  for (const [, raw] of blocks) {
    try {
      const data = JSON.parse(raw);
      const types = (data["@graph"] ?? [data]).map((n) => n["@type"]).filter(Boolean);
      pass(path, `JSON-LD parses (${types.join(", ")})`);
    } catch (e) {
      fail(path, `JSON-LD does not parse: ${e.message}`);
    }
  }

  return { path, title };
}

async function auditSitemap(paths) {
  const xml = LIVE
    ? await (await fetch(`${ORIGIN}/sitemap.xml`)).text()
    : await readFile(join(SITE, "sitemap.xml"), "utf8");
  const listed = [...xml.matchAll(/<loc>(.*?)<\/loc>/g)].map((m) =>
    m[1].replace(ORIGIN, "").trim()
  );

  for (const p of paths) {
    if (!listed.includes(p)) fail("sitemap.xml", `does not list ${p}`);
  }
  for (const l of listed) {
    if (!paths.includes(l)) fail("sitemap.xml", `lists ${l}, which is not a page`);
  }
  if (listed.length === paths.length && paths.every((p) => listed.includes(p)))
    pass("sitemap.xml", `${listed.length} pages, all present`);

  const robots = LIVE
    ? await (await fetch(`${ORIGIN}/robots.txt`)).text()
    : await readFile(join(SITE, "robots.txt"), "utf8");
  if (!robots.includes("Sitemap:")) fail("robots.txt", "does not point at the sitemap");
  else pass("robots.txt", "declares the sitemap");
}

/** Duplicate titles across pages compete with each other in results. */
function auditDuplicates(pages) {
  const seen = new Map();
  for (const { path, title } of pages) {
    if (!title) continue;
    if (seen.has(title)) fail(path, `duplicate title, same as ${seen.get(title)}`);
    else seen.set(title, path);
  }
}

async function auditAssets() {
  const dir = join(SITE, "assets");
  for (const entry of await readdir(dir)) {
    const { size } = await stat(join(dir, entry));
    const kb = Math.round(size / 1024);
    if (kb > ASSET_WARN_KB) warn("assets", `${entry} is ${kb} KB — check it is not blocking LCP`);
  }
}

const files = await htmlPages();
console.log(`\nSEO audit — ${LIVE ? `live (${ORIGIN})` : "local files"}, ${files.length} pages\n`);

const results = [];
for (const file of files) {
  const { path, html } = await loadPage(file);
  results.push(auditPage(path, html));
  console.log("");
}
auditDuplicates(results);
await auditSitemap(results.map((r) => r.path));
if (!LIVE) await auditAssets();

console.log(`\n${errors} error(s), ${warnings} warning(s)\n`);
process.exit(errors > 0 ? 1 : 0);

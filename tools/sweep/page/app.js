"use strict";

const ORDER = ["pure", "default"];
const COLORS = ["var(--accent)", "var(--warn)", "var(--ok)", "var(--bad)"];

const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) =>
  ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const pct = (x) => (x * 100).toFixed(1) + "%";
const groupOf = (r) => (r.os === "linux" ? r.mode : `${r.os}-${r.mode}`);
const keyOf = (r) => r.repo ?? `fixtures/${r.fixture}`;
const sortGroups = (gs) => [...gs].sort((a, b) => {
  const ia = ORDER.indexOf(a), ib = ORDER.indexOf(b);
  return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib) || a.localeCompare(b);
});

async function load(path) {
  try {
    const res = await fetch(path, { cache: "no-store" });
    return res.ok ? await res.json() : null;
  } catch {
    return null;
  }
}

function cards(results) {
  const el = document.getElementById("cards");
  el.innerHTML = sortGroups(Object.keys(results.summary)).map((g) => {
    const s = results.summary[g];
    const skipped = s.composer_failed + s.fetch_failed;
    return `<div class="card">
      <div class="label">${esc(g)}</div>
      <div class="big">${s.identical} of ${s.installable}</div>
      <div>identical <code>vendor/</code> (${pct(s.ratio)})</div>
      <div class="muted">native install: ${pct(s.native_rate)} · Composer failed: ${skipped}</div>
    </div>`;
  }).join("");
}

function history(entries) {
  if (!entries || entries.length === 0) return;
  const groups = sortGroups(new Set(entries.flatMap((e) => Object.keys(e.summary))));
  const w = 720, h = 220, padL = 44, padR = 12, padT = 12, padB = 28;
  const ratios = entries.flatMap((e) => groups.map((g) => e.summary[g]?.ratio).filter((x) => x != null));
  const lo = Math.max(0, Math.floor(Math.min(...ratios) * 10) / 10 - 0.05);
  const x = (i) => padL + (entries.length === 1 ? (w - padL - padR) / 2 : (i * (w - padL - padR)) / (entries.length - 1));
  const y = (r) => padT + (1 - (r - lo) / (1 - lo || 1)) * (h - padT - padB);
  let svg = `<svg class="chart" viewBox="0 0 ${w} ${h}" role="img" aria-label="Identical ratio per night">`;
  for (const t of [lo, (lo + 1) / 2, 1]) {
    svg += `<line x1="${padL}" x2="${w - padR}" y1="${y(t)}" y2="${y(t)}" stroke="var(--line)"/>`;
    svg += `<text x="${padL - 6}" y="${y(t) + 4}" text-anchor="end">${pct(t)}</text>`;
  }
  svg += `<text x="${x(0)}" y="${h - 8}">${esc(entries[0].date)}</text>`;
  if (entries.length > 1) {
    svg += `<text x="${x(entries.length - 1)}" y="${h - 8}" text-anchor="end">${esc(entries.at(-1).date)}</text>`;
  }
  groups.forEach((g, gi) => {
    const pts = entries.map((e, i) => (e.summary[g] ? [x(i), y(e.summary[g].ratio)] : null)).filter(Boolean);
    const c = COLORS[gi % COLORS.length];
    svg += `<polyline fill="none" stroke="${c}" stroke-width="2" points="${pts.map((p) => p.join(",")).join(" ")}"/>`;
    for (const [px, py] of pts) svg += `<circle cx="${px}" cy="${py}" r="3" fill="${c}"/>`;
  });
  svg += "</svg>";
  const legend = groups.map((g, gi) =>
    `<span style="color:${COLORS[gi % COLORS.length]}">&#9632;</span> ${esc(g)}`).join(" &nbsp; ");
  const el = document.getElementById("history");
  el.className = "";
  el.innerHTML = svg + `<p class="muted">${legend}</p>`;
}

function firstProblem(rs) {
  for (const r of rs) {
    if (r.outcome === "identical") continue;
    const run = r.outcome === "composer-failed" ? r.composer : r.phpm;
    const why = r.first_differences?.[0] ?? r.error ?? run?.stderr_tail?.split("\n").filter(Boolean).at(-1);
    return `${groupOf(r)}: ${why ?? r.outcome}`;
  }
  return "";
}

function table(results) {
  const groups = sortGroups(new Set(results.records.map(groupOf)));
  document.getElementById("mode-heads").outerHTML = groups.map((g) => `<th>${esc(g)}</th>`).join("");
  const projects = new Map();
  for (const r of results.records) {
    const k = keyOf(r);
    if (!projects.has(k)) projects.set(k, []);
    projects.get(k).push(r);
  }
  const rows = [...projects.entries()].map(([k, rs]) => {
    const by = Object.fromEntries(rs.map((r) => [groupOf(r), r]));
    const first = rs[0];
    const link = first.repo
      ? `<a href="https://github.com/${esc(first.repo)}/tree/${esc(first.commit)}">${esc(k)}</a>`
      : esc(k);
    const def = by.default;
    const path = def ? `${esc(def.phpm_path)}<div class="why">${esc(def.phpm_reasons.join("; "))}</div>` : "";
    const problem = firstProblem(rs);
    const cells = groups.map((g) => {
      const r = by[g];
      return r ? `<td><span class="tag ${esc(r.outcome)}">${esc(r.outcome)}</span>${
        r.differences ? ` <span class="why">${r.differences}</span>` : ""}</td>` : "<td></td>";
    }).join("");
    return { k, problem, html: `<tr><td>${link}</td><td class="num">${first.stars ?? ""}</td>
      <td class="num">${first.packages ?? ""}</td>${cells}<td>${path}</td>
      <td class="why">${esc(problem)}</td></tr>` };
  });
  const body = document.getElementById("rows");
  const filter = document.getElementById("filter");
  const problems = document.getElementById("problems");
  const render = () => {
    const q = filter.value.trim().toLowerCase();
    body.innerHTML = rows
      .filter((r) => (!q || r.k.toLowerCase().includes(q)) && (!problems.checked || r.problem))
      .map((r) => r.html).join("");
  };
  filter.addEventListener("input", render);
  problems.addEventListener("change", render);
  render();
}

(async () => {
  const [results, hist] = await Promise.all([load("results.json"), load("history.json")]);
  const meta = document.getElementById("meta");
  if (!results) {
    meta.textContent = "No results published yet.";
    return;
  }
  const m = results.meta ?? {};
  const sha = m.phpm ? ` · phpm <a href="https://github.com/speedsharmaai/phpm/commit/${esc(m.phpm)}"><code>${esc(m.phpm.slice(0, 7))}</code></a>` : "";
  const run = m.run ? ` · <a href="https://github.com/speedsharmaai/phpm/actions/runs/${esc(m.run)}">run</a>` : "";
  meta.innerHTML = `${esc(results.date)} · Composer ${esc(m.composer ?? "?")} · PHP ${esc(m.php ?? "?")}${sha}${run}`;
  cards(results);
  history(hist);
  table(results);
})();

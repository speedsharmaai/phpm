"use strict";

const OS_ORDER = ["ubuntu-latest", "macos-latest", "windows-latest"];
const ACCENT = "var(--accent)";
const GREY = "var(--ink-3)";

const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) =>
  ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
const pct = (x) => (x * 100).toFixed(1) + "%";
const secs = (s) => (s == null ? "" : s < 1 ? `${(s * 1000).toFixed(0)} ms` : `${s.toFixed(2)} s`);
const speedup = (t) => (t && t.composer_seconds > 0 && t.phpm_seconds > 0 ? t.composer_seconds / t.phpm_seconds : null);
const keyOf = (r) => r.repo ?? `fixtures/${r.fixture}`;
const nameOf = (r) => r.repo ?? `fixtures/${r.fixture}`;
const sortOs = (a, b) => {
  const ia = OS_ORDER.indexOf(a), ib = OS_ORDER.indexOf(b);
  return (ia < 0 ? 99 : ia) - (ib < 0 ? 99 : ib) || a.localeCompare(b);
};

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
  const oses = Object.keys(results.summary).sort(sortOs);
  el.innerHTML = oses.map((os) => {
    const s = results.summary[os];
    const warm = s.warm ? `${s.warm.median_speedup.toFixed(1)}x` : "&mdash;";
    return `<div class="card">
      <div class="label">${esc(os)}</div>
      <div class="big">${warm}</div>
      <div>median warm speed-up, n=${s.warm ? s.warm.n : 0}</div>
      <div class="muted">${s.identity_rate != null ? pct(s.identity_rate) : "&mdash;"} identical &middot; ${s.fallback_rate != null ? pct(s.fallback_rate) : "&mdash;"} fallback</div>
    </div>`;
  }).join("");
}

// Packagist's own monthly install count, cited in this project's README:
// ~3B/month in January 2026, over 5B/month by September. The per-install
// seconds-saved figure is real (today's median warm delta); the share of
// those installs that are warm, repeated ones phpm actually speeds up
// (CI caches, agent worktrees, local reinstalls) is a stated guess, not
// a measured number.
const PACKAGIST_INSTALLS_PER_MONTH = 5_000_000_000;
const ASSUMED_WARM_SHARE = 0.01;

function timeSaved(results) {
  const ubuntu = results.summary["ubuntu-latest"];
  const valueEl = document.getElementById("time-saved-value");
  const noteEl = document.getElementById("time-saved-assumption");
  if (!ubuntu || !ubuntu.warm) {
    valueEl.textContent = "not enough data yet";
    return;
  }
  const row = results.records.find((r) => r.os === "ubuntu-latest" && r.warm);
  const perInstall = row ? row.warm.composer_seconds - row.warm.phpm_seconds : null;
  if (perInstall == null || perInstall <= 0) {
    valueEl.textContent = "not enough data yet";
    return;
  }
  const installsPerYear = PACKAGIST_INSTALLS_PER_MONTH * 12;
  const secondsPerYear = installsPerYear * ASSUMED_WARM_SHARE * perInstall;
  const years = secondsPerYear / (365.25 * 24 * 3600);
  valueEl.textContent = `~${years.toFixed(1)} compute-years / year`;
  noteEl.innerHTML = `Packagist reports roughly ${(PACKAGIST_INSTALLS_PER_MONTH / 1e9).toFixed(0)} billion installs a month
    (see the project README). <strong>Assumption, stated plainly:</strong> ${(ASSUMED_WARM_SHARE * 100).toFixed(0)}%
    of those are warm, repeated installs like the ones measured here (CI caches, agent worktrees, local reinstalls) &mdash;
    a deliberately conservative guess, not a measured figure. At ${secs(perInstall)} saved per warm install
    (today's <code>ubuntu-latest</code> median), that is the number above. Change the assumption and recompute
    with real per-install numbers from <a href="results.json">results.json</a>.`;
}

function barChart(rows, os, pick) {
  const data = rows
    .filter((r) => r.os === os && pick(r))
    .map((r) => ({ name: nameOf(r), t: pick(r), s: speedup(pick(r)) }))
    .filter((d) => d.s != null)
    .sort((a, b) => b.s - a.s);
  if (data.length === 0) return "<p class=\"muted\">No data yet.</p>";
  const rowH = 34, labelW = 220, chartW = 560, gap = 2;
  const maxSeconds = Math.max(...data.map((d) => d.t.composer_seconds));
  const barScale = (chartW - 60) / maxSeconds;
  const h = data.length * rowH;
  let svg = `<svg class="bars" viewBox="0 0 ${labelW + chartW} ${h}" role="img" aria-label="Composer vs phpm, ${esc(os)}">`;
  data.forEach((d, i) => {
    const y = i * rowH;
    const cw = Math.max(2, d.t.composer_seconds * barScale);
    const pw = Math.max(2, d.t.phpm_seconds * barScale);
    const barH = (rowH - gap * 3) / 2;
    svg += `<text class="name" x="${labelW - 10}" y="${y + rowH / 2 + 4}">${esc(d.name)}</text>`;
    svg += `<rect x="${labelW}" y="${y + gap}" width="${chartW - 60}" height="${barH}" fill="var(--surface-2)" rx="3"/>`;
    svg += `<rect x="${labelW}" y="${y + gap}" width="${cw}" height="${barH}" fill="${GREY}" rx="3"/>`;
    svg += `<rect x="${labelW}" y="${y + gap * 2 + barH}" width="${chartW - 60}" height="${barH}" fill="var(--surface-2)" rx="3"/>`;
    svg += `<rect x="${labelW}" y="${y + gap * 2 + barH}" width="${pw}" height="${barH}" fill="${ACCENT}" rx="3"/>`;
    svg += `<text class="value" x="${labelW + pw + 6}" y="${y + gap * 2 + barH * 1.5 + 4}" fill="var(--accent)">${d.s.toFixed(1)}x</text>`;
  });
  svg += "</svg>";
  return svg;
}

function scenarioCharts(results) {
  const el = document.getElementById("scenario-charts");
  const scenarios = [["cold", (r) => r.cold], ["warm", (r) => r.warm], ["noop", (r) => r.noop]];
  el.innerHTML = scenarios.map(([name, pick]) => `
    <div class="chart-block">
      <div class="chart-title">${name}</div>
      ${barChart(results.records, "ubuntu-latest", pick)}
      <div class="legend"><span><span class="dot" style="background:${GREY}"></span>Composer</span>
        <span><span class="dot" style="background:${ACCENT}"></span>phpm</span></div>
    </div>`).join("");
}

function worktreeRows(results) {
  const body = document.getElementById("worktree-rows");
  const rows = [...results.worktree].sort((a, b) => sortOs(a.os, b.os));
  body.innerHTML = rows.map((w) => `<tr>
    <td>${esc(w.os)}</td><td class="num">${w.worktrees}</td>
    <td class="num">total</td><td class="num">${w.time_speedup.toFixed(1)}x faster</td>
    <td class="num">${w.time_speedup.toFixed(1)}x</td>
    <td class="num">${w.disk_ratio.toFixed(1)}x smaller on disk</td>
  </tr>`).join("") || `<tr><td colspan="6" class="muted">No data yet.</td></tr>`;
}

function ciRows(results) {
  const body = document.getElementById("ci-rows");
  const rows = [...results.ci_scenario].sort((a, b) => sortOs(a.os, b.os) || a.project.localeCompare(b.project));
  body.innerHTML = rows.map((c) => `<tr>
    <td>${esc(c.os)}</td><td>${esc(c.project)}</td>
    <td class="num">&mdash;</td><td class="num">&mdash;</td>
    <td class="num">${c.with_cache_speedup != null ? c.with_cache_speedup.toFixed(1) + "x" : "&mdash;"}</td>
    <td class="num">${c.without_cache_speedup != null ? c.without_cache_speedup.toFixed(1) + "x" : "&mdash;"}</td>
  </tr>`).join("") || `<tr><td colspan="6" class="muted">No data yet.</td></tr>`;
}

function badge(outcome) {
  const cls = outcome === "identical" ? "identical" : outcome === "different" ? "different" : "install-failed";
  return `<span class="badge ${cls}"><span class="dot"></span>${esc(outcome)}</span>`;
}

function table(results) {
  const rows = [...results.records].map((r) => {
    const fallback = r.fallback ? (r.fallback_plugins || []).join(", ") || "yes" : "no";
    return {
      key: keyOf(r),
      problem: r.identity !== "identical" || r.fallback,
      html: `<tr>
        <td>${r.repo ? `<a href="https://github.com/${esc(r.repo)}">${esc(nameOf(r))}</a>` : esc(nameOf(r))}</td>
        <td class="num">${r.stars ?? ""}</td><td class="num">${r.packages ?? ""}</td><td>${esc(r.os)}</td>
        <td class="num">${r.cold ? `${secs(r.cold.phpm_seconds)} <span class="why">(${speedup(r.cold)?.toFixed(1) ?? "?"}x)</span>` : ""}</td>
        <td class="num">${r.warm ? `${secs(r.warm.phpm_seconds)} <span class="why">(${speedup(r.warm)?.toFixed(1) ?? "?"}x)</span>` : ""}</td>
        <td class="num">${r.noop ? `${secs(r.noop.phpm_seconds)} <span class="why">(${speedup(r.noop)?.toFixed(1) ?? "?"}x)</span>` : ""}</td>
        <td>${badge(r.identity)}</td>
        <td class="why">${esc(fallback)}</td>
      </tr>`,
    };
  });
  const body = document.getElementById("rows");
  const filter = document.getElementById("filter");
  const problems = document.getElementById("problems");
  const render = () => {
    const q = filter.value.trim().toLowerCase();
    body.innerHTML = rows
      .filter((r) => (!q || r.key.toLowerCase().includes(q)) && (!problems.checked || r.problem))
      .map((r) => r.html).join("");
  };
  filter.addEventListener("input", render);
  problems.addEventListener("change", render);
  render();
}

(async () => {
  const results = await load("results.json");
  const meta = document.getElementById("meta");
  if (!results) {
    meta.textContent = "No results published yet.";
    return;
  }
  const m = results.meta ?? {};
  const sha = m.phpm ? ` &middot; phpm <a href="https://github.com/speedsharmaai/phpm/commit/${esc(m.phpm)}"><code>${esc(m.phpm.slice(0, 7))}</code></a>` : "";
  const run = m.run ? ` &middot; <a href="https://github.com/speedsharmaai/phpm/actions/runs/${esc(m.run)}">run</a>` : "";
  meta.innerHTML = `${esc(results.date)} &middot; Composer ${esc(m.composer ?? "?")} &middot; PHP ${esc(m.php ?? "?")}${sha}${run}`;
  cards(results);
  timeSaved(results);
  scenarioCharts(results);
  worktreeRows(results);
  ciRows(results);
  table(results);
})();

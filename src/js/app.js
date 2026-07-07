/* ------------------------------------------------------------------
   app.js — retro UI controller for envyou.
   ------------------------------------------------------------------ */
(function () {
  "use strict";

  const COLORS = ["#008080", "#000080", "#FF0000", "#808000", "#800080", "#008000", "#000000"];
  const FREE_MAX_PROJECTS = 3;
  const FREE_MAX_VARS = 10;
  // Pure developer-convenience logic (parse/export/generate/diff). Loaded from
  // devtools.js before this script.
  const DEV = window.EnvyouDev || {};

  const state = {
    data: null,
    selectedProjectId: null,
    revealAll: false, // session-only "show values" toggle
    lastError: "", // last mutation error, surfaced to modals without DOM round-tripping
    locked: false, // true while the master-password unlock gate is up
  };

  const $ = (sel) => document.querySelector(sel);

  // Make a click handler also fire on Enter/Space for keyboard users.
  const onActivate = (fn) => (e) => {
    if (e.type === "click" || e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      fn(e);
    }
  };
  const el = (tag, props = {}, children = []) => {
    const node = document.createElement(tag);
    Object.entries(props).forEach(([k, v]) => {
      if (k === "class") node.className = v;
      else if (k === "text") node.textContent = v;
      else if (k.startsWith("on") && typeof v === "function")
        node.addEventListener(k.slice(2).toLowerCase(), v);
      else node.setAttribute(k, v);
    });
    (Array.isArray(children) ? children : [children]).forEach((c) => {
      if (c == null) return;
      node.appendChild(typeof c === "string" ? document.createTextNode(c) : c);
    });
    return node;
  };

  // ---- i18n (core UI) ----
  const I18N = {
    en: { free:"FREE", upgrade:"Upgrade to Pro »", projects:"PROJECTS", variables:"VARIABLES",
      select_project:"Select or create a project.", no_projects:"No projects yet — click + to add one.",
      no_vars:"No variables. Click + to add one.", upgrade_title:"Upgrade to Pro — $59 lifetime",
      upgrade_feats:"Unlimited projects & variables, MCP server, custom env colors.",
      upgrade_buy:"Buy at envyou.dev — your license key arrives by email. Offline activation.",
      license_key:"License key", activate:"Activate", cancel:"Cancel",
      copy_env:"Copy .env", import_env:"Import .env",
      import_title:"Import from .env", import_hint:"Paste KEY=value lines (a .env file). Existing keys are updated; # comments are ignored.", import_btn:"Import",
      pro_only:"Pro feature", unlock_pro:"Unlock with Pro", lifetime_cta:"One-time $59 — yours forever. No subscription, no renewals.",
      locked_projects:"Free tier is capped at 3 projects.", locked_vars:"Free tier is capped at 10 variables per project.",
      locked_color:"Custom env colors are a Pro feature.", locked_mcp:"Claude Desktop (MCP) linking is a Pro feature." },
    ko: { free:"무료", upgrade:"Pro로 업그레이드 »", projects:"프로젝트", variables:"변수",
      select_project:"프로젝트를 선택하거나 만드세요.", no_projects:"아직 프로젝트가 없습니다 — +를 눌러 추가하세요.",
      no_vars:"변수가 없습니다. +를 눌러 추가하세요.", upgrade_title:"Pro 업그레이드 — $59 평생",
      upgrade_feats:"무제한 프로젝트·변수, MCP 서버, 커스텀 컬러.",
      upgrade_buy:"envyou.dev에서 구매 — 라이선스 키는 이메일로. 오프라인 인증.",
      license_key:"라이선스 키", activate:"활성화", cancel:"취소",
      copy_env:".env 복사", import_env:".env 가져오기",
      import_title:".env 가져오기", import_hint:"KEY=value 형식(.env)을 붙여넣으세요. 기존 키는 업데이트되고, # 주석은 무시됩니다.", import_btn:"가져오기",
      pro_only:"Pro 전용 기능", unlock_pro:"Pro로 잠금 해제", lifetime_cta:"한 번 결제 $59 — 평생 소장. 구독·갱신 없음.",
      locked_projects:"무료 버전은 프로젝트 3개까지입니다.", locked_vars:"무료 버전은 프로젝트당 변수 10개까지입니다.",
      locked_color:"커스텀 환경 컬러는 Pro 전용입니다.", locked_mcp:"Claude Desktop(MCP) 연동은 Pro 전용입니다." },
    ja: { free:"無料", upgrade:"Pro にアップグレード »", projects:"プロジェクト", variables:"変数",
      select_project:"プロジェクトを選択または作成してください。", no_projects:"プロジェクトがありません — + で追加。",
      no_vars:"変数がありません。+ で追加。", upgrade_title:"Pro にアップグレード — $59 買い切り",
      upgrade_feats:"無制限のプロジェクト・変数、MCPサーバー、カスタムカラー。",
      upgrade_buy:"envyou.dev で購入 — ライセンスキーはメールで届きます。オフライン認証。",
      license_key:"ライセンスキー", activate:"有効化", cancel:"キャンセル" },
    zh: { free:"免费", upgrade:"升级到 Pro »", projects:"项目", variables:"变量",
      select_project:"请选择或创建一个项目。", no_projects:"还没有项目 — 点击 + 添加。",
      no_vars:"没有变量。点击 + 添加。", upgrade_title:"升级到 Pro — $59 永久",
      upgrade_feats:"无限项目和变量、MCP 服务器、自定义颜色。",
      upgrade_buy:"在 envyou.dev 购买 — 许可证密钥通过邮件发送。离线激活。",
      license_key:"许可证密钥", activate:"激活", cancel:"取消" },
    th: { free:"ฟรี", upgrade:"อัปเกรดเป็น Pro »", projects:"โปรเจกต์", variables:"ตัวแปร",
      select_project:"เลือกหรือสร้างโปรเจกต์", no_projects:"ยังไม่มีโปรเจกต์ — กด + เพื่อเพิ่ม",
      no_vars:"ไม่มีตัวแปร กด + เพื่อเพิ่ม", upgrade_title:"อัปเกรดเป็น Pro — $59 ตลอดชีพ",
      upgrade_feats:"โปรเจกต์และตัวแปรไม่จำกัด, เซิร์ฟเวอร์ MCP, สีที่กำหนดเอง",
      upgrade_buy:"ซื้อที่ envyou.dev — คีย์ไลเซนส์ส่งทางอีเมล เปิดใช้งานออฟไลน์",
      license_key:"คีย์ไลเซนส์", activate:"เปิดใช้งาน", cancel:"ยกเลิก" },
    vi: { free:"MIỄN PHÍ", upgrade:"Nâng cấp Pro »", projects:"DỰ ÁN", variables:"BIẾN",
      select_project:"Chọn hoặc tạo một dự án.", no_projects:"Chưa có dự án — nhấn + để thêm.",
      no_vars:"Chưa có biến. Nhấn + để thêm.", upgrade_title:"Nâng cấp Pro — $59 trọn đời",
      upgrade_feats:"Dự án & biến không giới hạn, máy chủ MCP, màu tùy chỉnh.",
      upgrade_buy:"Mua tại envyou.dev — khóa giấy phép gửi qua email. Kích hoạt ngoại tuyến.",
      license_key:"Khóa giấy phép", activate:"Kích hoạt", cancel:"Hủy" },
    hi: { free:"फ़्री", upgrade:"Pro में अपग्रेड »", projects:"प्रोजेक्ट", variables:"वेरिएबल",
      select_project:"कोई प्रोजेक्ट चुनें या बनाएँ।", no_projects:"अभी कोई प्रोजेक्ट नहीं — जोड़ने के लिए + दबाएँ।",
      no_vars:"कोई वेरिएबल नहीं। जोड़ने के लिए + दबाएँ।", upgrade_title:"Pro में अपग्रेड — $59 आजीवन",
      upgrade_feats:"असीमित प्रोजेक्ट और वेरिएबल, MCP सर्वर, कस्टम रंग।",
      upgrade_buy:"envyou.dev पर खरीदें — लाइसेंस कुंजी ईमेल से आती है। ऑफ़लाइन सक्रियण।",
      license_key:"लाइसेंस कुंजी", activate:"सक्रिय करें", cancel:"रद्द करें" }
  };
  let LANG = "en";
  function t(k) {
    const d = I18N[LANG] || I18N.en;
    return d[k] != null ? d[k] : (I18N.en[k] != null ? I18N.en[k] : k);
  }
  function applyStaticI18n() {
    document.querySelectorAll("[data-i18n]").forEach((n) => {
      n.textContent = t(n.getAttribute("data-i18n"));
    });
  }
  function setLang(l) {
    LANG = I18N[l] ? l : "en";
    try { localStorage.setItem("envyou_lang", LANG); } catch (e) {}
    document.documentElement.setAttribute("lang", LANG);
    const sel = $("#lang-select");
    if (sel) sel.value = LANG;
    applyStaticI18n();
    if (state.data) render();
  }

  function status(msg) {
    $("#status-text").textContent = msg;
  }

  // ---- Data load / refresh --------------------------------------------------
  async function refresh() {
    try {
      state.data = await window.api.getState();
    } catch (e) {
      status("Error loading state: " + e);
      return;
    }
    if (
      state.selectedProjectId &&
      !state.data.projects.some((p) => p.id === state.selectedProjectId)
    ) {
      state.selectedProjectId = null;
    }
    if (!state.selectedProjectId && state.data.projects.length)
      state.selectedProjectId = state.data.projects[0].id;
    render();
  }

  function selectedProject() {
    return state.data?.projects.find((p) => p.id === state.selectedProjectId) || null;
  }

  // Free-tier gates, mirrored from the backend policy so the UI can show a
  // "Pro" lock *before* a rejected mutation. These are UX nudges only — the
  // real cap is still enforced in envyou-core.
  function isPro() {
    return !!(state.data && state.data.license && state.data.license.isPro);
  }
  function canAddProject() {
    return isPro() || (state.data && state.data.projects.length < FREE_MAX_PROJECTS);
  }
  function canAddVariable(p) {
    return isPro() || (!!p && p.variables.length < FREE_MAX_VARS);
  }
  // A small inline "PRO" badge for locked features shown to free users.
  function proBadge() {
    return el("span", {
      text: "PRO",
      title: t("pro_only"),
      style: "margin-left:6px;font-size:9px;font-weight:bold;padding:1px 4px;border:1px solid #000080;color:#000080;letter-spacing:.5px",
    });
  }

  // ---- Render ---------------------------------------------------------------
  function render() {
    renderTier();
    renderProjects();
    renderVariables();
  }

  function renderTier() {
    const pro = state.data.license.isPro;
    const label = $("#tier-label");
    label.textContent = pro ? "PRO ✦" : t("free");
    label.classList.toggle("pro", pro);
    $("#upgrade-btn").style.display = pro ? "none" : "inline";
  }

  function renderProjects() {
    const list = $("#project-list");
    list.innerHTML = "";
    const pro = state.data.license.isPro;
    // Free-tier usage counter on the panel head (e.g. "PROJECTS 2/3").
    const head = $("#projects-count");
    if (head) {
      const atCap = state.data.projects.length >= FREE_MAX_PROJECTS;
      head.textContent = pro ? "" : `${state.data.projects.length}/${FREE_MAX_PROJECTS}` + (atCap ? " 🔒" : "");
    }

    state.data.projects.forEach((p) => {
      const li = el("li", {
        class: p.id === state.selectedProjectId ? "active" : "",
        role: "button",
        tabindex: "0",
        "aria-label": `Project ${p.name}, ${p.variables.length} variables`,
        "aria-pressed": p.id === state.selectedProjectId ? "true" : "false",
        title: p.name,
        onclick: onActivate(() => {
          state.selectedProjectId = p.id;
          render();
        }),
        onkeydown: onActivate(() => {
          state.selectedProjectId = p.id;
          render();
        }),
      });
      li.style.borderLeftColor = p.colorTag;
      li.appendChild(el("span", { class: "swatch" }));
      li.lastChild.style.background = p.colorTag;
      li.appendChild(el("span", { class: "pname", text: p.name }));
      li.appendChild(el("span", { class: "count", text: String(p.variables.length) }));
      list.appendChild(li);
    });
    if (!state.data.projects.length) {
      list.appendChild(el("li", { class: "empty-hint", text: t("no_projects") }));
    }
  }

  function renderVariables() {
    const p = selectedProject();
    const c = $("#vars-container");
    c.innerHTML = "";
    $("#vars-title").textContent = p ? t("variables") + " — " + p.name : t("variables");

    if (!p) {
      c.appendChild(el("p", { class: "empty-hint", text: t("select_project") }));
      return;
    }
    if (!p.variables.length) {
      c.appendChild(el("p", { class: "empty-hint", text: t("no_vars") }));
      return;
    }

    // Free-tier variable counter (e.g. "8/10") on the panel head.
    const vc = $("#vars-count");
    if (vc) {
      const atCap = p.variables.length >= FREE_MAX_VARS;
      vc.textContent = state.data.license.isPro ? "" : `${p.variables.length}/${FREE_MAX_VARS}` + (atCap ? " 🔒" : "");
    }

    const maskGlobal = state.data.settings.maskSensitiveData && !state.revealAll;
    p.variables.forEach((v) => {
      const masked = maskGlobal && v.isMasked;
      const shown = masked ? "••••" : v.value;
      const row = el("div", { class: "var-row" }, [
        el("span", { class: "var-key", text: v.key, title: v.comment || v.key }),
        el("span", {
          class: "var-val",
          text: shown,
          role: "button",
          tabindex: "0",
          "aria-label": `Copy value of ${v.key}`,
          title: "Click to copy",
          onclick: onActivate(() => copyValue(v)),
          onkeydown: onActivate(() => copyValue(v)),
        }),
        el("span", { class: "var-actions" }, [
          el("button", { class: "mini-btn", title: "Copy value", "aria-label": `Copy ${v.key}`, text: "⧉", onclick: () => copyValue(v) }),
          el("button", { class: "mini-btn", title: "Edit", "aria-label": `Edit ${v.key}`, text: "✎", onclick: () => editVarModal(p, v) }),
          el("button", {
            class: "mini-btn",
            title: "Delete",
            "aria-label": `Delete ${v.key}`,
            text: "✕",
            onclick: () => deleteVar(p, v),
          }),
        ]),
      ]);
      c.appendChild(row);
    });
  }

  async function copyValue(v) {
    try {
      await navigator.clipboard.writeText(v.value);
      status("Copied " + v.key + " to clipboard");
    } catch {
      status("Copy failed (clipboard unavailable)");
    }
  }

  // Copy the whole project as a ready-to-paste .env block (KEY=value lines).
  function looksSensitiveKey(key) {
    return DEV.classifyKey ? DEV.classifyKey(key).isSensitive : /(SECRET|TOKEN|KEY|PASSWORD|PRIVATE)/i.test(key);
  }

  // ---- Smart Import: paste any format -> preview -> resolve -> apply --------
  function smartImportModal() {
    const p = selectedProject();
    if (!p) {
      status("Create or select a project first.");
      return;
    }
    const ta = el("textarea", { rows: "7", placeholder: "Paste .env, `export KEY=...`, JSON, or code using process.env.KEY" });
    const err = el("p", { class: "error-text" });
    const summary = el("p", { class: "hint" });
    const preview = el("div", { style: "max-height:210px;overflow:auto;margin-top:4px" });
    const actionsBar = el("div", { class: "modal-actions" });

    let parsed = null;
    const choices = new Map(); // index -> { action, newKey }

    const existingKeys = () => new Set((selectedProject()?.variables || []).map((v) => v.key));

    function renderPreview() {
      preview.innerHTML = "";
      const exist = existingKeys();
      if (!parsed || !parsed.entries.length) {
        preview.appendChild(el("p", { class: "empty-hint", text: "Nothing to preview yet." }));
        return;
      }
      parsed.entries.forEach((entry, i) => {
        const isUpdate = exist.has(entry.key);
        const tags = [];
        if (entry.isEmpty) tags.push("empty");
        if (entry.isPublic) tags.push("public");
        if (entry.isSensitive) tags.push("secret");
        if (entry.duplicate) tags.push("dup");

        const cur = choices.get(i) || { action: isUpdate ? "overwrite" : "add", newKey: "" };
        choices.set(i, cur);

        const sel = el("select", { style: "font-family:inherit;font-size:10px" });
        const opts = isUpdate
          ? [["overwrite", "overwrite"], ["skip", "keep existing"], ["rename", "save as…"]]
          : [["add", "add"], ["skip", "skip"], ["rename", "save as…"]];
        opts.forEach(([v, l]) => sel.appendChild(el("option", { value: v, text: l })));
        sel.value = cur.action;

        const rename = el("input", { type: "text", value: cur.newKey, placeholder: "NEW_KEY", style: "font-size:10px;width:110px;display:" + (cur.action === "rename" ? "inline-block" : "none") });
        rename.addEventListener("input", () => { cur.newKey = rename.value.trim(); });
        sel.addEventListener("change", () => { cur.action = sel.value; rename.style.display = cur.action === "rename" ? "inline-block" : "none"; });

        const shown = entry.isEmpty ? "(empty)" : looksSensitiveKey(entry.key) ? "••••" : entry.value;
        preview.appendChild(el("div", { class: "var-row", style: "gap:6px;align-items:center" }, [
          el("span", { text: isUpdate ? "UPDATE" : "NEW", style: "font-size:9px;font-weight:bold;color:" + (isUpdate ? "#800000" : "#006000") }),
          el("span", { class: "var-key", text: entry.key }),
          el("span", { class: "var-val", text: shown, style: "opacity:.7" }),
          el("span", { text: tags.join(" · "), style: "font-size:9px;opacity:.6" }),
          sel,
          rename,
        ]));
      });
    }

    function doParse() {
      err.textContent = "";
      parsed = DEV.parseEnvText(ta.value);
      choices.clear();
      if (!parsed.entries.length) {
        summary.textContent = "";
        err.textContent = "No variables found. Paste .env, export lines, or JSON.";
        renderPreview();
        return;
      }
      const exist = existingKeys();
      const news = parsed.entries.filter((e) => !exist.has(e.key)).length;
      summary.textContent = `${parsed.entries.length} parsed — ${news} new, ${parsed.entries.length - news} existing` + (parsed.ignored.length ? `, ${parsed.ignored.length} ignored` : "");
      renderPreview();
      renderActions(true);
    }

    async function doImport() {
      let added = 0, updated = 0, skipped = 0, blocked = 0;
      for (let i = 0; i < parsed.entries.length; i++) {
        const entry = parsed.entries[i];
        const c = choices.get(i) || { action: "add" };
        if (c.action === "skip") { skipped++; continue; }
        const proj = selectedProject();
        if (!proj) break;
        let key = entry.key;
        if (c.action === "rename") {
          if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(c.newKey || "")) { skipped++; continue; }
          key = c.newKey;
        }
        const exists = proj.variables.some((v) => v.key === key);
        if (!exists && !canAddVariable(proj)) { blocked++; continue; }
        const ok = await run(window.api.upsertVariable(proj.id, key, entry.value, null, looksSensitiveKey(key)));
        if (ok) exists ? updated++ : added++;
      }
      closeModal();
      status(`Imported ${added} new, updated ${updated}` + (skipped ? `, skipped ${skipped}` : "") + (blocked ? `, ${blocked} blocked by free cap` : ""));
      if (blocked > 0) proLockedModal("locked_vars");
    }

    function renderActions(hasPreview) {
      actionsBar.innerHTML = "";
      actionsBar.appendChild(el("button", { class: "btn", text: "Cancel", onclick: closeModal }));
      actionsBar.appendChild(el("button", { class: "btn", text: "Preview", onclick: doParse }));
      if (hasPreview) actionsBar.appendChild(el("button", { class: "btn primary", text: "Import", onclick: doImport }));
    }

    renderActions(false);
    openModal("Smart Import", [
      el("p", { class: "hint", text: "Paste any format — .env, export lines, JSON, or code using process.env.KEY. Review, resolve conflicts, then import." }),
      el("div", { class: "field" }, [ta]),
      summary,
      preview,
      err,
      actionsBar,
    ]);
    ta.focus();
  }

  // ---- Smart Export: many formats, masking, selection ----------------------
  function exportModal() {
    const p = selectedProject();
    if (!p || !p.variables.length) {
      status("No variables to export.");
      return;
    }
    const fmt = el("select", { style: "font-family:inherit" });
    [["dotenv", ".env"], ["dotenv-local", ".env.local"], ["shell", "export KEY=…"], ["json", "JSON"], ["docker", "Docker Compose"], ["gha", "GitHub Actions"], ["example", ".env.example"]]
      .forEach(([v, l]) => fmt.appendChild(el("option", { value: v, text: l })));
    const keysOnly = el("input", { type: "checkbox" });
    const maskVals = el("input", { type: "checkbox" });
    const out = el("textarea", { rows: "8", readonly: "readonly", style: "width:100%;font-size:11px" });
    const warn = el("p", { class: "error-text" });

    const varChecks = p.variables.map((v) => {
      const cb = el("input", { type: "checkbox" });
      cb.checked = true;
      return { cb, v };
    });
    const selectedKeys = () => varChecks.filter((x) => x.cb.checked).map((x) => x.v.key);
    const anySensitive = () => varChecks.some((x) => x.cb.checked && looksSensitiveKey(x.v.key));
    const isRaw = () => !keysOnly.checked && !maskVals.checked && fmt.value !== "gha" && fmt.value !== "example";

    function refresh() {
      out.value = DEV.formatExport(p.variables, fmt.value, { keysOnly: keysOnly.checked, maskValues: maskVals.checked, selectedKeys: selectedKeys() });
      warn.textContent = isRaw() && anySensitive() ? "⚠ This output contains raw secrets. Do not commit it to Git." : "";
    }
    [fmt, keysOnly, maskVals].forEach((c) => c.addEventListener("change", refresh));
    varChecks.forEach((x) => x.cb.addEventListener("change", refresh));

    const copy = async () => {
      if (isRaw() && anySensitive() && !confirm("This copies RAW secrets to your clipboard. Continue?")) return;
      try { await navigator.clipboard.writeText(out.value); status("Export copied to clipboard"); }
      catch { status("Copy failed (clipboard unavailable)"); }
    };
    const nameByFmt = { dotenv: ".env", "dotenv-local": ".env.local", shell: "env.sh", json: "env.json", docker: "compose-env.yml", gha: "env.yml", example: ".env.example" };
    const download = () => {
      if (isRaw() && anySensitive() && !confirm("This file will contain RAW secrets. Make sure it is not committed to Git. Continue?")) return;
      try {
        const url = URL.createObjectURL(new Blob([out.value], { type: "text/plain" }));
        const a = el("a", { href: url, download: nameByFmt[fmt.value] || "export.txt" });
        document.body.appendChild(a);
        a.click();
        a.remove();
        setTimeout(() => URL.revokeObjectURL(url), 1000);
        status("Saved " + (nameByFmt[fmt.value] || "export.txt"));
      } catch { status("Download unavailable — use Copy."); }
    };

    const varList = el("div", { style: "max-height:110px;overflow:auto;border:2px solid var(--dark-gray,#808080);padding:4px;margin:4px 0" },
      varChecks.map((x) => el("label", { class: "checkbox-row", style: "font-size:11px" }, [x.cb, document.createTextNode(" " + x.v.key)])));

    openModal("Export — " + p.name, [
      el("div", { class: "field" }, [el("label", { text: "Format" }), fmt]),
      el("label", { class: "checkbox-row" }, [keysOnly, document.createTextNode(" Keys only (no values)")]),
      el("label", { class: "checkbox-row" }, [maskVals, document.createTextNode(" Mask values (••••)")]),
      el("div", { class: "field" }, [el("label", { text: "Variables" }), varList]),
      out,
      warn,
      el("div", { class: "modal-actions" }, [
        el("button", { class: "btn", text: "Close", onclick: closeModal }),
        el("button", { class: "btn", text: "Download", onclick: download }),
        el("button", { class: "btn primary", text: "Copy", onclick: copy }),
      ]),
    ]);
    refresh();
  }

  // ---- Secret Generator ----------------------------------------------------
  function secretGenModal(onInsert) {
    const type = el("select", { style: "font-family:inherit" });
    [["hex", "Random hex"], ["base64", "Base64"], ["urlsafe", "URL-safe token"], ["uuid", "UUID v4"], ["password", "Strong password"], ["jwt", "JWT secret"]]
      .forEach(([v, l]) => type.appendChild(el("option", { value: v, text: l })));
    const length = el("input", { type: "number", value: "32", min: "8", max: "256", style: "width:80px" });
    const symbols = el("input", { type: "checkbox" });
    const out = el("input", { type: "text", readonly: "readonly", style: "width:100%;font-family:monospace" });

    const gen = () => {
      try { out.value = DEV.generateSecret(type.value, { length: parseInt(length.value, 10) || 32, symbols: symbols.checked }); }
      catch { out.value = ""; }
    };
    [type, length, symbols].forEach((c) => c.addEventListener("change", gen));
    const copy = async () => {
      try { await navigator.clipboard.writeText(out.value); status("Secret copied"); }
      catch { status("Copy failed"); }
    };
    const actions = [
      el("button", { class: "btn", text: "Close", onclick: closeModal }),
      el("button", { class: "btn", text: "Regenerate", onclick: gen }),
      el("button", { class: "btn", text: "Copy", onclick: copy }),
    ];
    if (onInsert) actions.push(el("button", { class: "btn primary", text: "Use value", onclick: () => { onInsert(out.value); closeModal(); } }));

    openModal("Secret Generator", [
      el("p", { class: "hint", text: "Generated locally with your OS secure random (CSPRNG). Never sent anywhere." }),
      el("div", { class: "field" }, [el("label", { text: "Type" }), type]),
      el("div", { class: "field" }, [el("label", { text: "Length (bytes / password chars)" }), length]),
      el("label", { class: "checkbox-row" }, [symbols, document.createTextNode(" Include symbols (password)")]),
      el("div", { class: "field" }, [el("label", { text: "Result" }), out]),
      el("div", { class: "modal-actions" }, actions),
    ]);
    gen();
  }

  // ---- Command Palette (Ctrl+K / "/") --------------------------------------
  function commandPalette() {
    const input = el("input", { type: "text", placeholder: "Search projects, variables, or actions…", "aria-label": "Command palette", style: "width:100%" });
    const listEl = el("ul", { style: "list-style:none;margin:6px 0 0;padding:0;max-height:260px;overflow:auto" });
    let items = [];
    let sel = 0;

    const acts = [
      { label: "＋ New variable", run: () => { const p = selectedProject(); if (!p) return status("Select a project first."); canAddVariable(p) ? editVarModal(p, null) : proLockedModal("locked_vars"); } },
      { label: "＋ New project", run: () => (canAddProject() ? projectModal(null) : proLockedModal("locked_projects")) },
      { label: "⇪ Smart Import", run: smartImportModal },
      { label: "⧉ Export", run: exportModal },
      { label: "⇄ Compare projects (Diff)", run: diffModal },
      { label: "🔑 Secret generator", run: () => secretGenModal(null) },
      { label: "≡ Settings", run: settingsModal },
    ];
    if (!isPro()) acts.push({ label: "★ Upgrade to Pro", run: () => upgradeModal() });
    const projs = (state.data.projects || []).map((p) => ({ label: "📁 " + p.name, run: () => { state.selectedProjectId = p.id; render(); } }));
    const vars = [];
    (state.data.projects || []).forEach((p) =>
      p.variables.forEach((v) =>
        vars.push({ label: "＄ " + v.key + "  (" + p.name + ")", run: async () => { try { await navigator.clipboard.writeText(v.value); status("Copied " + v.key); } catch { status("Copy failed"); } } })));
    const ALL = acts.concat(projs, vars);

    function draw() {
      const q = input.value.trim().toLowerCase();
      items = q ? ALL.filter((it) => it.label.toLowerCase().includes(q)) : ALL;
      if (sel >= items.length) sel = Math.max(0, items.length - 1);
      listEl.innerHTML = "";
      items.forEach((it, i) => {
        listEl.appendChild(el("li", {
          text: it.label,
          style: "padding:4px 6px;cursor:pointer;" + (i === sel ? "background:var(--navy,#000080);color:#fff" : ""),
          onclick: () => { closeModal(); it.run(); },
        }));
      });
      if (!items.length) listEl.appendChild(el("li", { class: "empty-hint", text: "No matches" }));
    }
    input.addEventListener("input", () => { sel = 0; draw(); });
    input.addEventListener("keydown", (e) => {
      if (e.key === "ArrowDown") { e.preventDefault(); sel = Math.min(items.length - 1, sel + 1); draw(); }
      else if (e.key === "ArrowUp") { e.preventDefault(); sel = Math.max(0, sel - 1); draw(); }
      else if (e.key === "Enter") { e.preventDefault(); const it = items[sel]; if (it) { closeModal(); it.run(); } }
    });

    openModal("Command Palette", [input, listEl]);
    draw();
    input.focus();
  }

  // ---- Diff: compare two projects ------------------------------------------
  function diffModal() {
    const projects = state.data.projects || [];
    if (projects.length < 2) {
      status("Need at least 2 projects to compare.");
      return;
    }
    const selA = el("select", { style: "font-family:inherit" });
    const selB = el("select", { style: "font-family:inherit" });
    projects.forEach((p) => {
      selA.appendChild(el("option", { value: p.id, text: p.name }));
      selB.appendChild(el("option", { value: p.id, text: p.name }));
    });
    selA.value = (selectedProject() || projects[0]).id;
    selB.value = (projects.find((p) => p.id !== selA.value) || projects[1]).id;
    let reveal = false;
    const result = el("div", { style: "max-height:230px;overflow:auto;margin-top:6px" });
    const proj = (id) => projects.find((p) => p.id === id);
    const shownVal = (val, key) => (reveal ? val : looksSensitiveKey(key) ? "••••" : val);

    function renderDiff() {
      const a = proj(selA.value), b = proj(selB.value);
      result.innerHTML = "";
      if (!a || !b || a.id === b.id) {
        result.appendChild(el("p", { class: "hint", text: "Pick two different projects." }));
        return;
      }
      const d = DEV.diffVarSets(a.variables, b.variables);
      const section = (title, rows) => {
        result.appendChild(el("div", { class: "panel-head" }, [el("span", { text: `${title} (${rows.length})` })]));
        if (!rows.length) result.appendChild(el("p", { class: "empty-hint", text: "—" }));
        else rows.forEach((r) => result.appendChild(r));
      };
      section("Only in " + a.name, d.onlyA.map((k) => {
        const av = a.variables.find((v) => v.key === k);
        return el("div", { class: "var-row" }, [
          el("span", { class: "var-key", text: k }),
          el("span", { class: "var-val", text: shownVal(av.value, k), style: "opacity:.7" }),
          el("button", { class: "mini-btn", title: "Copy to " + b.name, text: "→", onclick: async () => {
            if (!canAddVariable(b)) return proLockedModal("locked_vars");
            const ok = await run(window.api.upsertVariable(b.id, k, av.value, null, looksSensitiveKey(k)));
            if (ok) { status(`Copied ${k} → ${b.name}`); renderDiff(); }
          } }),
        ]);
      }));
      section("Only in " + b.name, d.onlyB.map((k) => {
        const bv = b.variables.find((v) => v.key === k);
        return el("div", { class: "var-row" }, [
          el("span", { class: "var-key", text: k }),
          el("span", { class: "var-val", text: shownVal(bv.value, k), style: "opacity:.7" }),
          el("button", { class: "mini-btn", title: "Copy to " + a.name, text: "←", onclick: async () => {
            if (!canAddVariable(a)) return proLockedModal("locked_vars");
            const ok = await run(window.api.upsertVariable(a.id, k, bv.value, null, looksSensitiveKey(k)));
            if (ok) { status(`Copied ${k} → ${a.name}`); renderDiff(); }
          } }),
        ]);
      }));
      section("Different values", d.changed.map((c) => el("div", { class: "var-row" }, [
        el("span", { class: "var-key", text: c.key }),
        el("span", { class: "var-val", text: `${shownVal(c.aValue, c.key)}  ≠  ${shownVal(c.bValue, c.key)}`, style: "opacity:.7;font-size:10px" }),
      ])));
      section("Identical", d.same.map((k) => el("div", { class: "var-row" }, [el("span", { class: "var-key", text: k })])));
    }

    const revealBtn = el("button", { class: "btn", text: "Show values", onclick: () => {
      const a = proj(selA.value), b = proj(selB.value);
      const prod = /prod/i.test((a && a.name) || "") || /prod/i.test((b && b.name) || "");
      if (!reveal && prod && !confirm("One of these looks like production. Reveal secret values?")) return;
      reveal = !reveal;
      revealBtn.textContent = reveal ? "Hide values" : "Show values";
      renderDiff();
    } });
    [selA, selB].forEach((s) => s.addEventListener("change", renderDiff));

    openModal("Compare projects", [
      el("div", { class: "field" }, [el("label", { text: "A" }), selA]),
      el("div", { class: "field" }, [el("label", { text: "B" }), selB]),
      el("div", { class: "modal-actions", style: "margin:4px 0" }, [revealBtn]),
      result,
      el("div", { class: "modal-actions" }, [el("button", { class: "btn", text: "Close", onclick: closeModal })]),
    ]);
    renderDiff();
  }

  // ---- Mutations ------------------------------------------------------------
  async function run(promise, okMsg) {
    try {
      state.data = await promise;
      state.lastError = "";
      render();
      if (okMsg) status(okMsg);
      return true;
    } catch (e) {
      // Keep the message on state so modals can show it directly instead of
      // scraping it back out of the status bar's DOM.
      state.lastError = String(e);
      status("✕ " + e);
      return false;
    }
  }

  async function deleteVar(p, v) {
    if (!confirm(`Delete variable "${v.key}"?`)) return;
    await run(window.api.deleteVariable(p.id, v.key), "Deleted " + v.key);
  }

  // ---- Modals ---------------------------------------------------------------
  let lastFocused = null;
  function openModal(title, contentNode, dismissible = true) {
    // Remember what had focus so we can restore it when the dialog closes.
    lastFocused = document.activeElement;
    const modal = $("#modal");
    modal.setAttribute("role", "dialog");
    modal.setAttribute("aria-modal", "true");
    modal.setAttribute("aria-label", title);
    modal.innerHTML = "";
    modal.appendChild(
      el("div", { class: "titlebar" }, [
        el("span", { class: "title", text: "■ " + title }),
        el("div", { class: "title-buttons" }, [
          dismissible
            ? el("button", { class: "title-btn", text: "✕", title: "Close", "aria-label": "Close dialog", onclick: closeModal })
            : null,
        ]),
      ])
    );
    modal.appendChild(el("div", { class: "modal-content" }, contentNode));
    $("#modal-overlay").classList.remove("hidden");
  }
  function closeModal() {
    // The unlock gate cannot be dismissed — there is no usable app behind it.
    if (state.locked) return;
    $("#modal-overlay").classList.add("hidden");
    // Restore focus to the control that opened the dialog.
    if (lastFocused && typeof lastFocused.focus === "function") lastFocused.focus();
    lastFocused = null;
  }

  // Keep Tab focus inside the open dialog (simple focus trap).
  function trapFocus(e) {
    if (e.key !== "Tab") return;
    if ($("#modal-overlay").classList.contains("hidden")) return;
    const focusable = $("#modal").querySelectorAll(
      'button, [href], input, textarea, select, [tabindex]:not([tabindex="-1"])'
    );
    if (!focusable.length) return;
    const first = focusable[0];
    const last = focusable[focusable.length - 1];
    if (e.shiftKey && document.activeElement === first) {
      e.preventDefault();
      last.focus();
    } else if (!e.shiftKey && document.activeElement === last) {
      e.preventDefault();
      first.focus();
    }
  }

  function colorPicker(initial, onPick, locked, onLockedClick) {
    let sel = initial || COLORS[0];
    const wrap = el("div", { class: "color-swatches" });
    if (locked) wrap.style.opacity = "0.55";
    const render = () => {
      wrap.innerHTML = "";
      COLORS.forEach((col) => {
        const sw = el("div", {
          class: "swatch-pick" + (col === sel ? " sel" : ""),
          title: locked ? t("locked_color") : col,
          onclick: () => {
            if (locked) {
              if (onLockedClick) onLockedClick();
              return;
            }
            sel = col;
            onPick(sel);
            render();
          },
        });
        sw.style.background = col;
        if (locked) sw.style.cursor = "not-allowed";
        wrap.appendChild(sw);
      });
    };
    render();
    return { node: wrap, get: () => sel };
  }

  function projectModal(existing) {
    const nameInput = el("input", { type: "text", value: existing ? existing.name : "", placeholder: "Project name" });
    const pro = isPro();
    const picker = colorPicker(
      existing ? existing.colorTag : COLORS[0],
      () => {},
      !pro,
      () => proLockedModal("locked_color")
    );
    const err = el("p", { class: "error-text" });

    const save = async () => {
      const name = nameInput.value.trim();
      if (!name) {
        err.textContent = "Name is required.";
        return;
      }
      // Snapshot existing ids so we can identify the newly-created project by id
      // rather than assuming it is last in the returned array (the backend may
      // order projects differently).
      const priorIds = new Set(state.data.projects.map((p) => p.id));
      const ok = existing
        ? await run(window.api.renameProject(existing.id, name, picker.get()), "Saved")
        : await run(window.api.createProject(name, picker.get()), "Project created");
      if (ok) {
        if (!existing) {
          const created = state.data.projects.find((p) => !priorIds.has(p.id));
          if (created) state.selectedProjectId = created.id;
          render();
        }
        closeModal();
      } else {
        err.textContent = state.lastError;
      }
    };

    openModal(existing ? "Edit Project" : "New Project", [
      el("div", { class: "field" }, [el("label", { text: "Name" }), nameInput]),
      el("div", { class: "field" }, [el("label", {}, ["Border color (env tag)", pro ? null : proBadge()]), picker.node]),
      err,
      el("div", { class: "modal-actions" }, [
        existing
          ? el("button", {
              class: "btn",
              text: "Delete",
              onclick: async () => {
                if (confirm(`Delete project "${existing.name}" and all its variables?`)) {
                  await run(window.api.deleteProject(existing.id), "Project deleted");
                  closeModal();
                }
              },
            })
          : null,
        el("button", { class: "btn", text: "Cancel", onclick: closeModal }),
        el("button", { class: "btn primary", text: "Save", onclick: save }),
      ]),
    ]);
    nameInput.focus();
  }

  // `draft` carries unsaved field values when the editor is re-opened after the
  // Secret Generator (the single modal host can't nest dialogs).
  function editVarModal(project, existing, draft) {
    const d = draft || {};
    const keyInput = el("input", {
      type: "text",
      value: d.key != null ? d.key : existing ? existing.key : "",
      placeholder: "DATABASE_URL",
    });
    if (existing) keyInput.setAttribute("readonly", "readonly");
    const valInput = el("textarea", { rows: "3", placeholder: "value" });
    valInput.value = d.value != null ? d.value : existing ? existing.value : "";
    const commentInput = el("input", {
      type: "text",
      value: d.comment != null ? d.comment : existing && existing.comment ? existing.comment : "",
      placeholder: "optional comment",
    });
    const maskCheck = el("input", { type: "checkbox" });
    maskCheck.checked = d.mask != null ? d.mask : existing ? existing.isMasked : true;
    const err = el("p", { class: "error-text" });

    const save = async () => {
      const key = keyInput.value.trim();
      if (!key) {
        err.textContent = "Key is required.";
        return;
      }
      const ok = await run(
        window.api.upsertVariable(project.id, key, valInput.value, commentInput.value.trim() || null, maskCheck.checked),
        "Saved " + key
      );
      if (ok) closeModal();
      else err.textContent = state.lastError;
    };

    openModal(existing ? "Edit Variable" : "New Variable", [
      el("div", { class: "field" }, [el("label", { text: "Key" }), keyInput]),
      el("div", { class: "field" }, [
        el("label", {}, [
          document.createTextNode("Value  "),
          el("button", {
            type: "button",
            class: "mini-btn",
            title: "Generate a secret",
            text: "🔑 Generate",
            style: "font-size:10px",
            onclick: () =>
              secretGenModal((val) =>
                editVarModal(project, existing, { key: keyInput.value, value: val, comment: commentInput.value, mask: maskCheck.checked })
              ),
          }),
        ]),
        valInput,
      ]),
      el("div", { class: "field" }, [el("label", { text: "Comment" }), commentInput]),
      el("label", { class: "checkbox-row" }, [maskCheck, document.createTextNode(" Mask on screen (••••)")]),
      err,
      el("div", { class: "modal-actions" }, [
        el("button", { class: "btn", text: "Cancel", onclick: closeModal }),
        el("button", { class: "btn primary", text: "Save", onclick: save }),
      ]),
    ]);
    (existing ? valInput : keyInput).focus();
  }

  function settingsModal() {
    const s = state.data.settings;
    const pro = isPro();
    const hotkey = el("input", { type: "text", value: s.globalHotkey });
    const aot = el("input", { type: "checkbox" });
    aot.checked = s.alwaysOnTop;
    const mask = el("input", { type: "checkbox" });
    mask.checked = s.maskSensitiveData;
    const claudeMsg = el("p", { class: "hint" });

    const save = async () => {
      const newSettings = {
        globalHotkey: hotkey.value.trim() || "Ctrl+Shift+E",
        alwaysOnTop: aot.checked,
        maskSensitiveData: mask.checked,
      };
      const ok = await run(window.api.saveSettings(newSettings), "Settings saved");
      if (ok) {
        window.api.setAlwaysOnTop(newSettings.alwaysOnTop);
        updatePinButton();
        closeModal();
      }
    };

    openModal("Settings", [
      el("div", { class: "field" }, [el("label", { text: "Global hotkey" }), hotkey]),
      el("label", { class: "checkbox-row" }, [aot, document.createTextNode(" Always on top")]),
      el("label", { class: "checkbox-row" }, [mask, document.createTextNode(" Mask sensitive values")]),
      el("hr"),
      el("div", { class: "field" }, [
        el("label", { text: "Encryption" }),
        el("button", {
          class: "btn",
          text: "Set master password »",
          onclick: () => masterPasswordModal(),
        }),
        el("p", { class: "hint", text: "Add an Argon2id master password on top of device encryption." }),
      ]),
      el("hr"),
      el("div", { class: "field" }, [
        el("label", {}, ["Claude Desktop (MCP)", pro ? null : proBadge()]),
        el("p", { class: "hint", text: "Let Claude read & write these variables — but only when you approve each request. One click adds envyou to claude_desktop_config.json (your other MCP servers are kept)." }),
        el("button", {
          class: "btn",
          text: pro ? "Link with Claude Desktop »" : "🔒 Link with Claude Desktop",
          onclick: pro
            ? async () => {
                try {
                  const where = await window.api.linkClaudeDesktop();
                  claudeMsg.textContent = "✔ Added to " + where + " — restart Claude Desktop, then ask it to \"list my envyou projects\".";
                } catch (e) {
                  claudeMsg.textContent = "✕ " + e;
                }
              }
            : () => proLockedModal("locked_mcp"),
        }),
        claudeMsg,
      ]),
      el("div", { class: "modal-actions" }, [
        el("button", { class: "btn", text: "Cancel", onclick: closeModal }),
        el("button", { class: "btn primary", text: "Save", onclick: save }),
      ]),
    ]);
  }

  // ---- Master password: unlock gate & setup --------------------------------
  function showUnlock() {
    state.locked = true;
    const pw = el("input", { type: "password", placeholder: "master password", "aria-label": "Master password" });
    const err = el("p", { class: "error-text" });
    const submit = async () => {
      try {
        state.data = await window.api.unlockVault(pw.value);
        state.locked = false;
        closeModal();
        if (state.data.projects.length) state.selectedProjectId = state.data.projects[0].id;
        render();
        updatePinButton();
        status("Vault unlocked");
      } catch (e) {
        err.textContent = String(e);
        pw.value = "";
        pw.focus();
      }
    };
    pw.addEventListener("keydown", (e) => {
      if (e.key === "Enter") submit();
    });
    openModal(
      "Unlock envyou",
      [
        el("p", { class: "hint", text: "This vault is protected by a master password. Enter it to continue." }),
        el("div", { class: "field" }, [el("label", { text: "Master password" }), pw]),
        err,
        el("div", { class: "modal-actions" }, [
          el("button", { class: "btn primary", text: "Unlock", onclick: submit }),
        ]),
      ],
      false // not dismissible
    );
    pw.focus();
  }

  function masterPasswordModal() {
    const pw = el("input", { type: "password", placeholder: "at least 8 characters", "aria-label": "New master password" });
    const pw2 = el("input", { type: "password", placeholder: "confirm password", "aria-label": "Confirm master password" });
    const err = el("p", { class: "error-text" });
    const save = async () => {
      if (pw.value !== pw2.value) {
        err.textContent = "Passwords do not match.";
        return;
      }
      const ok = await run(window.api.setMasterPassword(pw.value), "Master password set — vault re-encrypted");
      if (ok) closeModal();
      else err.textContent = state.lastError;
    };
    openModal("Set Master Password", [
      el("p", { class: "hint", text: "Re-encrypts your vault with Argon2id. You'll enter this password each time envyou starts. It is never stored." }),
      el("div", { class: "field" }, [el("label", { text: "New password" }), pw]),
      el("div", { class: "field" }, [el("label", { text: "Confirm" }), pw2]),
      err,
      el("div", { class: "modal-actions" }, [
        el("button", { class: "btn", text: "Cancel", onclick: closeModal }),
        el("button", { class: "btn primary", text: "Set password", onclick: save }),
      ]),
    ]);
    pw.focus();
  }

  // Shown when a free user clicks a locked Pro feature. Names the feature, then
  // drops them straight into the upgrade flow with the lifetime pitch.
  function proLockedModal(lockNoteKey) {
    upgradeModal(lockNoteKey);
  }

  function upgradeModal(lockNoteKey) {
    if (state.data.license.isPro) {
      openModal("Pro License", [
        el("p", { text: "You are on the Pro tier ✦" }),
        el("p", { class: "hint", text: "Key: " + (state.data.license.licenseKey || "—") }),
        el("div", { class: "modal-actions" }, [el("button", { class: "btn primary", text: "OK", onclick: closeModal })]),
      ]);
      return;
    }
    const keyInput = el("input", { type: "text", placeholder: "paste your license key", "aria-label": "License key" });
    const err = el("p", { class: "error-text" });
    const activate = async () => {
      const ok = await run(window.api.activateLicense(keyInput.value), "Pro activated! ✦");
      if (ok) closeModal();
      else err.textContent = state.lastError;
    };
    const buy = () => {
      const url = "https://envyou.dev/#pricing";
      try {
        if (window.api && typeof window.api.openExternal === "function") window.api.openExternal(url);
        else window.open(url, "_blank");
      } catch {
        status("Visit envyou.dev to buy Pro.");
      }
    };
    openModal(t("upgrade_title"), [
      // If a specific locked feature triggered this, name it up top.
      lockNoteKey ? el("p", { text: "🔒 " + t(lockNoteKey), style: "font-weight:bold;margin:0 0 6px" }) : null,
      el("p", { class: "hint", text: t("upgrade_feats") }),
      el("p", { text: t("lifetime_cta"), style: "font-weight:bold;margin:6px 0;color:#000080" }),
      el("div", { class: "modal-actions", style: "margin:8px 0" }, [
        el("button", { class: "btn primary", text: "★ " + t("unlock_pro") + " — $59", onclick: buy }),
      ]),
      el("hr"),
      el("div", { class: "field" }, [el("label", { text: t("license_key") }), keyInput]),
      el("p", { class: "hint", text: t("upgrade_buy") }),
      err,
      el("div", { class: "modal-actions" }, [
        el("button", { class: "btn", text: t("cancel"), onclick: closeModal }),
        el("button", { class: "btn primary", text: t("activate"), onclick: activate }),
      ]),
    ]);
    keyInput.focus();
  }

  // ---- Title bar controls ---------------------------------------------------
  let pinned = true;
  function updatePinButton() {
    pinned = state.data ? state.data.settings.alwaysOnTop : true;
    const btn = $("#pin-btn");
    btn.classList.toggle("active", pinned);
    btn.setAttribute("aria-pressed", pinned ? "true" : "false");
  }
  async function togglePin() {
    pinned = !pinned;
    const s = { ...state.data.settings, alwaysOnTop: pinned };
    await run(window.api.saveSettings(s));
    window.api.setAlwaysOnTop(pinned);
    updatePinButton();
    status(pinned ? "Pinned on top" : "Unpinned");
  }

  function minimize() {
    if (window.__TAURI__ && window.__TAURI__.window) {
      window.__TAURI__.window.getCurrentWindow().hide();
    } else {
      status("Minimize (preview): hides to tray on desktop.");
    }
  }

  // ---- Wire up --------------------------------------------------------------
  async function init() {
    // Language selector (core UI i18n).
    const langSel = $("#lang-select");
    let savedLang = null;
    try { savedLang = localStorage.getItem("envyou_lang"); } catch (e) {}
    if (langSel) langSel.addEventListener("change", () => setLang(langSel.value));
    setLang(savedLang || (navigator.language || "en").slice(0, 2).toLowerCase());

    $("#add-project-btn").addEventListener("click", () => {
      if (!canAddProject()) {
        proLockedModal("locked_projects");
        return;
      }
      projectModal(null);
    });
    $("#add-var-btn").addEventListener("click", () => {
      const p = selectedProject();
      if (!p) {
        status("Create or select a project first.");
        return;
      }
      if (!canAddVariable(p)) {
        proLockedModal("locked_vars");
        return;
      }
      editVarModal(p, null);
    });
    $("#copy-env-btn").addEventListener("click", exportModal);
    $("#import-env-btn").addEventListener("click", smartImportModal);
    $("#diff-btn").addEventListener("click", diffModal);
    $("#cmd-btn").addEventListener("click", commandPalette);
    $("#mask-toggle-btn").addEventListener("click", () => {
      state.revealAll = !state.revealAll;
      $("#mask-toggle-btn").classList.toggle("active", state.revealAll);
      renderVariables();
      status(state.revealAll ? "Values revealed" : "Values masked");
    });
    $("#settings-btn").addEventListener("click", settingsModal);
    $("#upgrade-btn").addEventListener("click", upgradeModal);
    $("#pin-btn").addEventListener("click", togglePin);
    $("#min-btn").addEventListener("click", minimize);
    $("#modal-overlay").addEventListener("click", (e) => {
      if (e.target.id === "modal-overlay") closeModal();
    });
    document.addEventListener("keydown", (e) => {
      if (e.key === "Escape") {
        closeModal();
        return;
      }
      // Command palette: Ctrl/Cmd+K anywhere, or "/" when not typing in a field.
      const modalOpen = !$("#modal-overlay").classList.contains("hidden");
      const typing = /^(input|textarea|select)$/i.test((e.target.tagName || ""));
      if ((e.key === "k" && (e.ctrlKey || e.metaKey)) || (e.key === "/" && !typing && !modalOpen)) {
        if (state.locked || !state.data) return;
        e.preventDefault();
        commandPalette();
        return;
      }
      trapFocus(e);
    });

    // Gate on the vault lock state before loading any data.
    let vs = { passwordProtected: false, unlocked: true };
    try {
      vs = await window.api.vaultStatus();
    } catch (e) {
      status("Could not read vault status: " + e);
    }
    if (vs.passwordProtected && !vs.unlocked) {
      if (!window.api.inTauri) status("Browser preview (mock data)");
      showUnlock();
      return;
    }
    await refresh();
    updatePinButton();
    if (!window.api.inTauri) status("Browser preview (mock data)");
  }

  document.addEventListener("DOMContentLoaded", init);
})();

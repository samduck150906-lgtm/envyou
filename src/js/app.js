/* ------------------------------------------------------------------
   app.js — retro UI controller for envyou.
   ------------------------------------------------------------------ */
(function () {
  "use strict";

  const COLORS = ["#008080", "#000080", "#FF0000", "#808000", "#800080", "#008000", "#000000"];
  const FREE_MAX_PROJECTS = 3;
  const FREE_MAX_VARS = 10;

  const state = {
    data: null,
    selectedProjectId: null,
    revealAll: false, // session-only "show values" toggle
    lastError: "", // last mutation error, surfaced to modals without DOM round-tripping
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

  // ---- Render ---------------------------------------------------------------
  function render() {
    renderTier();
    renderProjects();
    renderVariables();
  }

  function renderTier() {
    const pro = state.data.license.isPro;
    const label = $("#tier-label");
    label.textContent = pro ? "PRO ✦" : "FREE";
    label.classList.toggle("pro", pro);
    $("#upgrade-btn").style.display = pro ? "none" : "inline";
  }

  function renderProjects() {
    const list = $("#project-list");
    list.innerHTML = "";
    const pro = state.data.license.isPro;
    // Free-tier usage counter on the panel head (e.g. "PROJECTS 2/3").
    const head = $("#projects-count");
    if (head) head.textContent = pro ? "" : `${state.data.projects.length}/${FREE_MAX_PROJECTS}`;

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
      list.appendChild(el("li", { class: "empty-hint", text: "No projects yet — click + to add one." }));
    }
  }

  function renderVariables() {
    const p = selectedProject();
    const c = $("#vars-container");
    c.innerHTML = "";
    $("#vars-title").textContent = p ? "VARIABLES — " + p.name : "VARIABLES";

    if (!p) {
      c.appendChild(el("p", { class: "empty-hint", text: "Select or create a project." }));
      return;
    }
    if (!p.variables.length) {
      c.appendChild(el("p", { class: "empty-hint", text: "No variables. Click + to add one." }));
      return;
    }

    // Free-tier variable counter (e.g. "8/10") on the panel head.
    const vc = $("#vars-count");
    if (vc) vc.textContent = state.data.license.isPro ? "" : `${p.variables.length}/${FREE_MAX_VARS}`;

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
  function openModal(title, contentNode) {
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
          el("button", { class: "title-btn", text: "✕", title: "Close", "aria-label": "Close dialog", onclick: closeModal }),
        ]),
      ])
    );
    modal.appendChild(el("div", { class: "modal-content" }, contentNode));
    $("#modal-overlay").classList.remove("hidden");
  }
  function closeModal() {
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

  function colorPicker(initial, onPick) {
    let sel = initial || COLORS[0];
    const wrap = el("div", { class: "color-swatches" });
    const render = () => {
      wrap.innerHTML = "";
      COLORS.forEach((col) => {
        const sw = el("div", {
          class: "swatch-pick" + (col === sel ? " sel" : ""),
          title: col,
          onclick: () => {
            sel = col;
            onPick(sel);
            render();
          },
        });
        sw.style.background = col;
        wrap.appendChild(sw);
      });
    };
    render();
    return { node: wrap, get: () => sel };
  }

  function projectModal(existing) {
    const nameInput = el("input", { type: "text", value: existing ? existing.name : "", placeholder: "Project name" });
    const picker = colorPicker(existing ? existing.colorTag : COLORS[0], () => {});
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
      el("div", { class: "field" }, [el("label", { text: "Border color (env tag)" }), picker.node]),
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

  function editVarModal(project, existing) {
    const keyInput = el("input", {
      type: "text",
      value: existing ? existing.key : "",
      placeholder: "DATABASE_URL",
    });
    if (existing) keyInput.setAttribute("readonly", "readonly");
    const valInput = el("textarea", { rows: "3", placeholder: "value" });
    valInput.value = existing ? existing.value : "";
    const commentInput = el("input", {
      type: "text",
      value: existing && existing.comment ? existing.comment : "",
      placeholder: "optional comment",
    });
    const maskCheck = el("input", { type: "checkbox" });
    maskCheck.checked = existing ? existing.isMasked : true;
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
      el("div", { class: "field" }, [el("label", { text: "Value" }), valInput]),
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
        el("label", { text: "Claude Desktop (MCP)" }),
        el("button", {
          class: "btn",
          text: "Link with Claude Desktop »",
          onclick: async () => {
            try {
              const where = await window.api.linkClaudeDesktop();
              claudeMsg.textContent = "Linked: " + where;
            } catch (e) {
              claudeMsg.textContent = "✕ " + e;
            }
          },
        }),
        claudeMsg,
      ]),
      el("div", { class: "modal-actions" }, [
        el("button", { class: "btn", text: "Cancel", onclick: closeModal }),
        el("button", { class: "btn primary", text: "Save", onclick: save }),
      ]),
    ]);
  }

  function upgradeModal() {
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
    openModal("Upgrade to Pro — $9.99 one-time", [
      el("p", { class: "hint", text: "Unlimited projects & variables, MCP server, custom env colors." }),
      el("div", { class: "field" }, [el("label", { text: "License key" }), keyInput]),
      el("p", { class: "hint", text: "Purchased online — key arrives by email. Offline activation." }),
      err,
      el("div", { class: "modal-actions" }, [
        el("button", { class: "btn", text: "Cancel", onclick: closeModal }),
        el("button", { class: "btn primary", text: "Activate", onclick: activate }),
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
  function init() {
    $("#add-project-btn").addEventListener("click", () => projectModal(null));
    $("#add-var-btn").addEventListener("click", () => {
      const p = selectedProject();
      if (!p) {
        status("Create or select a project first.");
        return;
      }
      editVarModal(p, null);
    });
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
      if (e.key === "Escape") closeModal();
      else trapFocus(e);
    });

    refresh().then(() => {
      updatePinButton();
      if (!window.api.inTauri) status("Browser preview (mock data)");
    });
  }

  document.addEventListener("DOMContentLoaded", init);
})();

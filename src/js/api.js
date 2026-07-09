/* ------------------------------------------------------------------
   api.js — thin wrapper over Tauri commands.

   When running inside the Tauri webview, calls are forwarded to the Rust
   backend via window.__TAURI__. When opened in a plain browser (for UI
   preview/development), a localStorage-backed mock mirrors the backend so the
   retro UI is fully demoable without compiling the desktop shell.
   ------------------------------------------------------------------ */
(function () {
  "use strict";

  const inTauri = !!(window.__TAURI__ && window.__TAURI__.core);

  // ---- Tauri-backed API -----------------------------------------------------
  function tauriInvoke(cmd, args) {
    return window.__TAURI__.core.invoke(cmd, args);
  }

  async function tauriSetAlwaysOnTop(enabled) {
    return tauriInvoke("set_always_on_top", { enabled });
  }

  // ---- Browser mock ---------------------------------------------------------
  const MOCK_KEY = "envyou_mock_state";
  const FREE_MAX_PROJECTS = 3;
  const FREE_MAX_VARS = 10;

  function uuid() {
    return "xxxxxxxx-xxxx-4xxx-yxxx-xxxxxxxxxxxx".replace(/[xy]/g, (c) => {
      const r = (Math.random() * 16) | 0;
      const v = c === "x" ? r : (r & 0x3) | 0x8;
      return v.toString(16);
    });
  }
  function nowIso() {
    return new Date().toISOString().replace(/\.\d+Z$/, "Z");
  }
  function loadMock() {
    const raw = localStorage.getItem(MOCK_KEY);
    if (raw) return JSON.parse(raw);
    return {
      version: "1.0.0",
      license: { isPro: false, licenseKey: null, activatedAt: null },
      settings: { globalHotkey: "Ctrl+Shift+E", alwaysOnTop: true, maskSensitiveData: true },
      projects: [],
    };
  }
  function saveMock(s) {
    localStorage.setItem(MOCK_KEY, JSON.stringify(s));
    return s;
  }
  function findProject(s, id) {
    return s.projects.find((p) => p.id === id);
  }

  const mock = {
    get_state: async () => loadMock(),
    create_project: async ({ name, colorTag }) => {
      const s = loadMock();
      if (!s.license.isPro && s.projects.length >= FREE_MAX_PROJECTS)
        throw "Free tier allows up to 3 projects. Upgrade to Pro for unlimited.";
      s.projects.push({ id: uuid(), name, colorTag, createdAt: nowIso(), variables: [] });
      return saveMock(s);
    },
    delete_project: async ({ projectId }) => {
      const s = loadMock();
      s.projects = s.projects.filter((p) => p.id !== projectId);
      return saveMock(s);
    },
    rename_project: async ({ projectId, name, colorTag }) => {
      const s = loadMock();
      const p = findProject(s, projectId);
      if (!p) throw "project not found";
      p.name = name;
      p.colorTag = colorTag;
      return saveMock(s);
    },
    upsert_variable: async ({ projectId, key, value, comment, isMasked }) => {
      const s = loadMock();
      const p = findProject(s, projectId);
      if (!p) throw "project not found";
      const existing = p.variables.find((v) => v.key === key);
      if (!existing && !s.license.isPro && p.variables.length >= FREE_MAX_VARS)
        throw "Free tier allows up to 10 variables per project. Upgrade to Pro.";
      if (existing) {
        existing.value = value;
        existing.comment = comment ?? null;
        existing.isMasked = isMasked;
      } else {
        p.variables.push({ key, value, comment: comment ?? null, isMasked });
      }
      return saveMock(s);
    },
    delete_variable: async ({ projectId, key }) => {
      const s = loadMock();
      const p = findProject(s, projectId);
      if (p) p.variables = p.variables.filter((v) => v.key !== key);
      return saveMock(s);
    },
    save_settings: async ({ settings }) => {
      const s = loadMock();
      s.settings = settings;
      return saveMock(s);
    },
    activate_pro: async ({ email, code }) => {
      // Browser preview only: the real app calls the Supabase activation RPC in
      // Rust, verifies the returned certificate offline, and stores it. Here we
      // just sanity-check the inputs so the preview can demo the Pro flow.
      const codeAlnum = (code || "").replace(/[^A-Za-z0-9]/g, "");
      if (!email || !email.includes("@") || codeAlnum.length < 8)
        throw "Please enter your license email and code.";
      const s = loadMock();
      s.license = { isPro: true, licenseKey: "PREVIEW_CERTIFICATE", activatedAt: nowIso() };
      return saveMock(s);
    },
    activate_certificate: async ({ certificate }) => {
      // Advanced paste path — sanity-check the <payload>.<signature> shape only.
      const parts = (certificate || "").trim().split(".");
      const ok = parts.length === 2 && parts[0].length > 0 && parts[1].length > 0;
      if (!ok) throw "This certificate is not valid.";
      const s = loadMock();
      s.license = { isPro: true, licenseKey: certificate.trim(), activatedAt: nowIso() };
      return saveMock(s);
    },
    link_claude_desktop: async () =>
      "(browser preview) Claude Desktop config would be merged on the desktop app.",
    // Browser preview has no real vault lock — always unlocked, no password.
    vault_status: async () => ({ exists: true, passwordProtected: false, unlocked: true }),
    unlock_vault: async () => loadMock(),
    set_master_password: async ({ password }) => {
      if ((password || "").trim().length < 8)
        throw "master password must be at least 8 characters";
      return loadMock();
    },
  };

  // ---- Public surface -------------------------------------------------------
  window.api = {
    inTauri,
    getState: () => (inTauri ? tauriInvoke("get_state") : mock.get_state()),
    createProject: (name, colorTag) =>
      inTauri ? tauriInvoke("create_project", { name, colorTag }) : mock.create_project({ name, colorTag }),
    deleteProject: (projectId) =>
      inTauri ? tauriInvoke("delete_project", { projectId }) : mock.delete_project({ projectId }),
    renameProject: (projectId, name, colorTag) =>
      inTauri
        ? tauriInvoke("rename_project", { projectId, name, colorTag })
        : mock.rename_project({ projectId, name, colorTag }),
    upsertVariable: (projectId, key, value, comment, isMasked) =>
      inTauri
        ? tauriInvoke("upsert_variable", { projectId, key, value, comment, isMasked })
        : mock.upsert_variable({ projectId, key, value, comment, isMasked }),
    deleteVariable: (projectId, key) =>
      inTauri ? tauriInvoke("delete_variable", { projectId, key }) : mock.delete_variable({ projectId, key }),
    saveSettings: (settings) =>
      inTauri ? tauriInvoke("save_settings", { settings }) : mock.save_settings({ settings }),
    activatePro: (email, code) =>
      inTauri ? tauriInvoke("activate_pro", { email, code }) : mock.activate_pro({ email, code }),
    activateCertificate: (certificate) =>
      inTauri
        ? tauriInvoke("activate_certificate", { certificate })
        : mock.activate_certificate({ certificate }),
    linkClaudeDesktop: () =>
      inTauri ? tauriInvoke("link_claude_desktop") : mock.link_claude_desktop(),
    setAlwaysOnTop: (enabled) => (inTauri ? tauriSetAlwaysOnTop(enabled) : Promise.resolve()),
    vaultStatus: () => (inTauri ? tauriInvoke("vault_status") : mock.vault_status()),
    unlockVault: (password) =>
      inTauri ? tauriInvoke("unlock_vault", { password }) : mock.unlock_vault({ password }),
    setMasterPassword: (password) =>
      inTauri
        ? tauriInvoke("set_master_password", { password })
        : mock.set_master_password({ password }),
  };
})();

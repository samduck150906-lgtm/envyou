/* ------------------------------------------------------------------
   devtools.js — pure, side-effect-free developer-convenience logic for
   envyou: multi-format .env parsing, export formatting, secret
   generation, and variable diffing.

   These functions never touch the encrypted vault or the network. They
   operate on plain key/value pairs that the frontend already holds (the
   app decrypts values for display). Keeping them pure means they run
   identically in the Tauri webview and in the browser preview, and they
   can be unit-tested under Node without a UI.

   Dual export: attaches to `window.EnvyouDev` in a browser and to
   `module.exports` under Node/CommonJS (for tests and the future CLI).
   ------------------------------------------------------------------ */
(function () {
  "use strict";

  // ---- key classification --------------------------------------------------
  // Heuristics only — used to nudge masking and warn on export. Never a
  // security boundary.
  const PUBLIC_RE = /^(NEXT_PUBLIC_|PUBLIC_|VITE_|REACT_APP_|EXPO_PUBLIC_)/i;
  const SENSITIVE_RE = /(SECRET|TOKEN|PASSWORD|PASSWD|PRIVATE|CREDENTIAL|API[_-]?KEY|_KEY$|^KEY$)/i;

  function classifyKey(key) {
    const k = String(key || "");
    const isPublic = PUBLIC_RE.test(k);
    const isSensitive =
      !isPublic &&
      (SENSITIVE_RE.test(k) || k.toUpperCase() === "DATABASE_URL" || /(^|_)KEY(_|$)/i.test(k));
    return { isPublic, isSensitive };
  }

  // ---- .env / export / JSON / js parsing -----------------------------------
  function stripQuotes(v) {
    if (v.length >= 2) {
      const a = v[0];
      const b = v[v.length - 1];
      if ((a === '"' && b === '"') || (a === "'" && b === "'") || (a === "`" && b === "`")) {
        return v.slice(1, -1);
      }
    }
    return v;
  }

  // Parse many "paste it and go" formats into a normalized entry list.
  // Returns { entries: [{key, value, isEmpty, isPublic, isSensitive,
  // suggestedMask, duplicate}], ignored: [line...] }. Later duplicates win
  // but are flagged.
  function parseEnvText(text) {
    const raw = String(text == null ? "" : text);
    const ignored = [];
    const order = [];
    const map = new Map();

    const push = (key, value) => {
      const cls = classifyKey(key);
      const duplicate = map.has(key);
      const entry = {
        key,
        value: value == null ? "" : String(value),
        isEmpty: value == null || String(value) === "",
        isPublic: cls.isPublic,
        isSensitive: cls.isSensitive,
        suggestedMask: cls.isSensitive && !cls.isPublic,
        duplicate,
      };
      if (duplicate) {
        map.set(key, entry); // last value wins
      } else {
        map.set(key, entry);
        order.push(key);
      }
    };

    const trimmed = raw.trim();

    // 1. JSON object of { KEY: value }.
    if (trimmed.startsWith("{")) {
      try {
        const obj = JSON.parse(trimmed);
        if (obj && typeof obj === "object" && !Array.isArray(obj)) {
          Object.keys(obj).forEach((k) => {
            const v = obj[k];
            push(k, typeof v === "object" && v !== null ? JSON.stringify(v) : v);
          });
          return { entries: order.map((k) => map.get(k)), ignored };
        }
      } catch (e) {
        // fall through to line parsing
      }
    }

    // 2. Line-based: dotenv, `export KEY=...`, and `process.env.KEY` refs.
    const envRef = /(?:process\.env|import\.meta\.env)\.([A-Za-z_][A-Za-z0-9_]*)/;
    raw.split(/\r?\n/).forEach((line0) => {
      let line = line0.trim();
      if (!line) return;
      if (line.startsWith("#") || line.startsWith("//")) return; // full-line comment

      // strip common leading tokens from pasted code
      line = line.replace(/^(export|const|let|var)\s+/i, "");

      const eq = line.indexOf("=");
      if (eq > 0) {
        let key = line.slice(0, eq).trim();
        // a trailing ":" (yaml-ish "KEY:") or type annotations are not valid
        key = key.replace(/[:]+$/, "").trim();
        if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(key)) {
          ignored.push(line0);
          return;
        }
        let val = line.slice(eq + 1).trim();
        val = val.replace(/[;,]\s*$/, ""); // drop trailing js/json punctuation
        // `const x = process.env.OPENAI_API_KEY` — the RHS is an env reference,
        // so the variable the user cares about is the referenced key (no value).
        const rhsRef = val.match(/^(?:process\.env|import\.meta\.env)\.([A-Za-z_][A-Za-z0-9_]*)$/);
        if (rhsRef) {
          push(rhsRef[1], "");
          return;
        }
        const wasQuoted = /^["'`]/.test(val);
        val = stripQuotes(val);
        if (!wasQuoted) {
          // strip an inline comment that follows whitespace
          const c = val.search(/\s#/);
          if (c >= 0) val = val.slice(0, c).trim();
        }
        push(key, val);
        return;
      }

      // no '=' — maybe a `process.env.KEY` reference (empty value)
      const m = line.match(envRef);
      if (m) {
        push(m[1], "");
        return;
      }
      ignored.push(line0);
    });

    return { entries: order.map((k) => map.get(k)), ignored };
  }

  // ---- export formatting ---------------------------------------------------
  const EXPORT_FORMATS = ["dotenv", "dotenv-local", "shell", "json", "docker", "gha", "example"];

  function yamlQuote(v) {
    if (v === "") return '""';
    if (/[:#{}\[\],&*?|<>=!%@`"'\\\s]/.test(v)) return JSON.stringify(v);
    return v;
  }

  // Render a variable list in a chosen format.
  // vars: [{key, value, isMasked, comment?, isPublic?, isSensitive?}]
  // opts: { keysOnly, maskValues, selectedKeys }
  function formatExport(vars, format, opts) {
    const o = opts || {};
    let list = Array.isArray(vars) ? vars.slice() : [];
    if (Array.isArray(o.selectedKeys)) {
      const sel = new Set(o.selectedKeys);
      list = list.filter((v) => sel.has(v.key));
    }
    const MASK = "••••••••";
    const valueOf = (v) => {
      if (o.keysOnly) return "";
      if (o.maskValues) return MASK;
      return v.value == null ? "" : String(v.value);
    };

    switch (format) {
      case "dotenv":
      case "dotenv-local": {
        return (
          list
            .map((v) => {
              const c = v.comment ? `# ${v.comment}\n` : "";
              return `${c}${v.key}=${valueOf(v)}`;
            })
            .join("\n") + (list.length ? "\n" : "")
        );
      }
      case "shell": {
        return (
          list
            .map((v) => {
              const val = valueOf(v);
              return `export ${v.key}="${val.replace(/"/g, '\\"')}"`;
            })
            .join("\n") + (list.length ? "\n" : "")
        );
      }
      case "json": {
        const obj = {};
        list.forEach((v) => (obj[v.key] = valueOf(v)));
        return JSON.stringify(obj, null, 2) + "\n";
      }
      case "docker": {
        const body = list.map((v) => `  ${v.key}: ${yamlQuote(valueOf(v))}`).join("\n");
        return `environment:\n${body}\n`;
      }
      case "gha": {
        // GitHub Actions: reference repo secrets, never raw values — safe by
        // construction.
        const body = list
          .map((v) => `  ${v.key}: \${{ secrets.${v.key} }}`)
          .join("\n");
        return `env:\n${body}\n`;
      }
      case "example": {
        // .env.example: keys with placeholder/empty values + required/public
        // annotations. Public keys keep their value; everything else is blank.
        return (
          list
            .map((v) => {
              const cls = {
                isPublic: v.isPublic != null ? v.isPublic : classifyKey(v.key).isPublic,
                isSensitive: v.isSensitive != null ? v.isSensitive : classifyKey(v.key).isSensitive,
              };
              const tag = cls.isPublic ? "# Public" : cls.isSensitive ? "# Required (secret)" : "# Required";
              const val = cls.isPublic ? (v.value == null ? "" : String(v.value)) : "";
              return `${tag}\n${v.key}=${val}`;
            })
            .join("\n\n") + (list.length ? "\n" : "")
        );
      }
      default:
        throw new Error("unknown export format: " + format);
    }
  }

  // ---- secret generation ---------------------------------------------------
  function randomBytes(n) {
    if (typeof window !== "undefined" && window.crypto && window.crypto.getRandomValues) {
      const a = new Uint8Array(n);
      window.crypto.getRandomValues(a);
      return a;
    }
    // Node (tests / CLI). `require` is never reached in the browser.
    // eslint-disable-next-line
    const nodeCrypto = require("crypto");
    return new Uint8Array(nodeCrypto.randomBytes(n));
  }

  function toHex(bytes) {
    let s = "";
    for (const b of bytes) s += b.toString(16).padStart(2, "0");
    return s;
  }
  function toB64(bytes) {
    let bin = "";
    for (const b of bytes) bin += String.fromCharCode(b);
    if (typeof btoa === "function") return btoa(bin);
    return Buffer.from(bytes).toString("base64");
  }
  function toB64Url(bytes) {
    return toB64(bytes).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
  }
  function uuidV4() {
    const b = randomBytes(16);
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    const h = toHex(b);
    return `${h.slice(0, 8)}-${h.slice(8, 12)}-${h.slice(12, 16)}-${h.slice(16, 20)}-${h.slice(20)}`;
  }

  const SECRET_TYPES = ["hex", "base64", "urlsafe", "uuid", "password", "jwt"];

  // Generate a secret using the platform CSPRNG. `length` is bytes for
  // hex/base64/urlsafe/jwt and characters for password.
  function generateSecret(type, opts) {
    const o = opts || {};
    switch (type) {
      case "hex":
        return toHex(randomBytes(Math.max(1, o.length || 32)));
      case "base64":
        return toB64(randomBytes(Math.max(1, o.length || 32)));
      case "urlsafe":
        return toB64Url(randomBytes(Math.max(1, o.length || 32)));
      case "uuid":
        return uuidV4();
      case "jwt":
        // 32 random bytes, base64 — a strong HMAC/JWT signing secret.
        return toB64(randomBytes(32));
      case "password": {
        const len = Math.max(8, o.length || 24);
        const lower = "abcdefghijkmnpqrstuvwxyz";
        const upper = "ABCDEFGHJKLMNPQRSTUVWXYZ";
        const digits = "23456789";
        const symbols = "!@#$%^&*()-_=+[]{}";
        let charset = lower + upper + digits;
        if (o.symbols) charset += symbols;
        const bytes = randomBytes(len);
        let out = "";
        for (let i = 0; i < len; i++) out += charset[bytes[i] % charset.length];
        return out;
      }
      default:
        throw new Error("unknown secret type: " + type);
    }
  }

  // ---- diff ----------------------------------------------------------------
  // Compare two variable lists (e.g. two projects, or two environments).
  // Returns { onlyA, onlyB, changed:[{key,aValue,bValue}], same } — keys only,
  // plus values for the "changed" set so the UI can mask them.
  function diffVarSets(aVars, bVars) {
    const a = new Map((aVars || []).map((v) => [v.key, v.value == null ? "" : String(v.value)]));
    const b = new Map((bVars || []).map((v) => [v.key, v.value == null ? "" : String(v.value)]));
    const onlyA = [];
    const onlyB = [];
    const changed = [];
    const same = [];
    for (const [k, av] of a) {
      if (!b.has(k)) onlyA.push(k);
      else if (b.get(k) !== av) changed.push({ key: k, aValue: av, bValue: b.get(k) });
      else same.push(k);
    }
    for (const k of b.keys()) {
      if (!a.has(k)) onlyB.push(k);
    }
    onlyA.sort();
    onlyB.sort();
    same.sort();
    changed.sort((x, y) => (x.key < y.key ? -1 : 1));
    return { onlyA, onlyB, changed, same };
  }

  const API = {
    classifyKey,
    parseEnvText,
    formatExport,
    generateSecret,
    diffVarSets,
    EXPORT_FORMATS,
    SECRET_TYPES,
  };

  if (typeof module !== "undefined" && module.exports) module.exports = API;
  if (typeof window !== "undefined") window.EnvyouDev = API;
})();

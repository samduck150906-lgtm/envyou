/* Node unit tests for src/js/devtools.js — the pure developer-convenience
   logic (parse / export / generate / diff). Run: `node test/devtools.test.js`.
   No test framework needed; exits non-zero on the first failure. */
const D = require("../src/js/devtools.js");

let pass = 0;
let fail = 0;
function ok(name, cond, extra) {
  if (cond) {
    pass++;
  } else {
    fail++;
    console.log(`FAIL ${name}${extra ? "  [" + extra + "]" : ""}`);
  }
}
const eq = (name, a, b) => ok(name, a === b, `${JSON.stringify(a)} !== ${JSON.stringify(b)}`);

// ---- parse: dotenv -------------------------------------------------------
{
  const r = D.parseEnvText("DATABASE_URL=postgres://x\nPORT=8080\n# comment\n\nEMPTY=");
  eq("dotenv count", r.entries.length, 3);
  eq("dotenv key1", r.entries[0].key, "DATABASE_URL");
  eq("dotenv val1", r.entries[0].value, "postgres://x");
  ok("dotenv DATABASE_URL sensitive", r.entries[0].isSensitive);
  ok("dotenv EMPTY isEmpty", r.entries[2].isEmpty);
}
// ---- parse: export + quotes ---------------------------------------------
{
  const r = D.parseEnvText("export DATABASE_URL=\"postgres://x\"\nexport OPENAI_API_KEY='sk-abc'");
  eq("export count", r.entries.length, 2);
  eq("export strips dq", r.entries[0].value, "postgres://x");
  eq("export strips sq", r.entries[1].value, "sk-abc");
  ok("API_KEY sensitive", r.entries[1].isSensitive);
}
// ---- parse: JSON ---------------------------------------------------------
{
  const r = D.parseEnvText('{ "DATABASE_URL": "postgres://x", "PORT": "8080" }');
  eq("json count", r.entries.length, 2);
  eq("json val", r.entries[0].value, "postgres://x");
}
// ---- parse: process.env refs (empty value) ------------------------------
{
  const r = D.parseEnvText("const x = process.env.OPENAI_API_KEY\nimport.meta.env.VITE_URL");
  eq("ref count", r.entries.length, 2);
  ok("ref1 empty", r.entries[0].isEmpty);
  eq("ref1 key", r.entries[0].key, "OPENAI_API_KEY");
  ok("VITE_ is public", r.entries[1].isPublic);
}
// ---- parse: public classification & suggestedMask -----------------------
{
  const r = D.parseEnvText("NEXT_PUBLIC_APP_URL=https://e.com\nSTRIPE_SECRET_KEY=sk_live_x");
  ok("NEXT_PUBLIC public", r.entries[0].isPublic);
  ok("NEXT_PUBLIC not masked", r.entries[0].suggestedMask === false);
  ok("SECRET suggestedMask", r.entries[1].suggestedMask === true);
}
// ---- parse: duplicate flag (last wins) ----------------------------------
{
  const r = D.parseEnvText("A=1\nA=2");
  eq("dup collapses", r.entries.length, 1);
  eq("dup last wins", r.entries[0].value, "2");
  ok("dup flagged", r.entries[0].duplicate === true);
}
// ---- parse: ignores junk lines ------------------------------------------
{
  const r = D.parseEnvText("this is not a var\nOK=1");
  eq("ignored kept", r.entries.length, 1);
  ok("ignored recorded", r.ignored.length === 1);
}

// ---- export: dotenv ------------------------------------------------------
{
  const vars = [{ key: "A", value: "1" }, { key: "B", value: "2" }];
  eq("dotenv export", D.formatExport(vars, "dotenv", {}), "A=1\nB=2\n");
}
// ---- export: shell -------------------------------------------------------
{
  const vars = [{ key: "A", value: 'x"y' }];
  eq("shell export escapes", D.formatExport(vars, "shell", {}), 'export A="x\\"y"\n');
}
// ---- export: json --------------------------------------------------------
{
  const vars = [{ key: "A", value: "1" }];
  eq("json export", D.formatExport(vars, "json", {}), '{\n  "A": "1"\n}\n');
}
// ---- export: docker & gha ------------------------------------------------
{
  const vars = [{ key: "DATABASE_URL", value: "postgres://x" }];
  ok("docker export", D.formatExport(vars, "docker", {}).startsWith("environment:\n  DATABASE_URL:"));
  eq("gha references secret", D.formatExport(vars, "gha", {}), "env:\n  DATABASE_URL: ${{ secrets.DATABASE_URL }}\n");
}
// ---- export: keysOnly / mask / selected ---------------------------------
{
  const vars = [{ key: "A", value: "1" }, { key: "B", value: "2" }];
  eq("keysOnly", D.formatExport(vars, "dotenv", { keysOnly: true }), "A=\nB=\n");
  ok("mask hides value", !/1/.test(D.formatExport(vars, "dotenv", { maskValues: true })));
  eq("selected only", D.formatExport(vars, "dotenv", { selectedKeys: ["B"] }), "B=2\n");
}
// ---- export: .env.example ------------------------------------------------
{
  const vars = [
    { key: "NEXT_PUBLIC_APP_URL", value: "https://e.com" },
    { key: "STRIPE_SECRET_KEY", value: "sk_live_x" },
  ];
  const ex = D.formatExport(vars, "example", {});
  ok("example keeps public value", /NEXT_PUBLIC_APP_URL=https:\/\/e\.com/.test(ex));
  ok("example blanks secret", /STRIPE_SECRET_KEY=\n/.test(ex) || /STRIPE_SECRET_KEY=$/.test(ex.trim()));
  ok("example no raw secret", !/sk_live_x/.test(ex));
}

// ---- secrets -------------------------------------------------------------
{
  eq("hex length", D.generateSecret("hex", { length: 16 }).length, 32);
  ok("hex is hex", /^[0-9a-f]+$/.test(D.generateSecret("hex", { length: 16 })));
  ok("uuid v4 shape", /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(D.generateSecret("uuid")));
  ok("urlsafe no +/=", !/[+/=]/.test(D.generateSecret("urlsafe", { length: 24 })));
  eq("password length", D.generateSecret("password", { length: 20 }).length, 20);
  ok("password symbols honored", /[!@#$%^&*()\-_=+\[\]{}]/.test(D.generateSecret("password", { length: 64, symbols: true })));
  ok("two secrets differ", D.generateSecret("hex") !== D.generateSecret("hex"));
  ok("jwt non-empty", D.generateSecret("jwt").length > 0);
}

// ---- diff ----------------------------------------------------------------
{
  const a = [{ key: "A", value: "1" }, { key: "B", value: "2" }, { key: "C", value: "3" }];
  const b = [{ key: "B", value: "2" }, { key: "C", value: "9" }, { key: "D", value: "4" }];
  const d = D.diffVarSets(a, b);
  eq("diff onlyA", d.onlyA.join(","), "A");
  eq("diff onlyB", d.onlyB.join(","), "D");
  eq("diff same", d.same.join(","), "B");
  eq("diff changed key", d.changed[0].key, "C");
  eq("diff changed aVal", d.changed[0].aValue, "3");
  eq("diff changed bVal", d.changed[0].bValue, "9");
}

console.log(`\n${pass} passed, ${fail} failed`);
process.exit(fail === 0 ? 0 : 1);

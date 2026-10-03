// Temporary: find t("key") calls whose key is missing from en/translation.json.
import fs from "node:fs";
import path from "node:path";
import ts from "typescript";

const en = JSON.parse(
  fs.readFileSync("src/i18n/locales/en/translation.json", "utf8"),
);
const flat = {};
(function f(o, p = "") {
  for (const [k, v] of Object.entries(o)) {
    const key = p ? `${p}.${k}` : k;
    if (v && typeof v === "object") f(v, key);
    else flat[key] = v;
  }
})(en);
const has = (k) =>
  k in flat ||
  Object.keys(flat).some((x) => x.startsWith(k + "_")) ||
  Object.keys(flat).some((x) => x.startsWith(k + "."));

const files = [];
(function walk(d) {
  for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, e.name);
    if (e.isDirectory()) {
      if (e.name !== "i18n") walk(p);
    } else if (/\.tsx?$/.test(e.name) && !/\.d\.ts$/.test(e.name))
      files.push(p);
  }
})("src");

const missing = [];
const dynamic = [];
const withDefault = [];
for (const file of files) {
  const src = fs.readFileSync(file, "utf8");
  const sf = ts.createSourceFile(
    file,
    src,
    ts.ScriptTarget.Latest,
    true,
    file.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
  const visit = (node) => {
    if (ts.isCallExpression(node)) {
      const callee = node.expression.getText(sf);
      if (/^(t|i18n\.t|i18next\.t)$/.test(callee) && node.arguments.length) {
        const a = node.arguments[0];
        const line =
          sf.getLineAndCharacterOfPosition(node.getStart(sf)).line + 1;
        const loc = `${file}:${line}`;
        if (ts.isStringLiteral(a) || ts.isNoSubstitutionTemplateLiteral(a)) {
          if (!has(a.text)) missing.push(`${loc}  ${a.text}`);
          const b = node.arguments[1];
          if (
            b &&
            (ts.isStringLiteral(b) ||
              ts.isNoSubstitutionTemplateLiteral(b) ||
              (ts.isObjectLiteralExpression(b) &&
                b.properties.some(
                  (p) => p.name?.getText(sf) === "defaultValue",
                )))
          )
            withDefault.push(`${loc}  ${a.text}`);
        } else {
          dynamic.push(`${loc}  ${a.getText(sf)}`);
        }
      }
    }
    if (ts.isJsxAttribute(node) && node.name.getText(sf) === "i18nKey") {
      const init = node.initializer;
      if (init && ts.isStringLiteral(init) && !has(init.text))
        missing.push(`${file}  i18nKey ${init.text}`);
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
}
console.log("MISSING", missing.length);
console.log(missing.join("\n"));
console.log("\nWITH DEFAULT", withDefault.length);
console.log("\nDYNAMIC", dynamic.length);
console.log(dynamic.join("\n"));

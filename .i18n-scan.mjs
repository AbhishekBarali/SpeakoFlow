// Temporary: find English UI strings that bypass t().
import fs from "node:fs";
import path from "node:path";
import ts from "typescript";

const ROOT = path.join(process.cwd(), "src");
const files = [];
(function walk(d) {
  for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, e.name);
    if (e.isDirectory()) {
      if (["i18n", "assets"].includes(e.name)) continue;
      walk(p);
    } else if (/\.(tsx?|)$/.test(e.name) && /\.tsx?$/.test(e.name)) {
      if (/\.test\.tsx?$|bindings\.ts$|\.d\.ts$/.test(e.name)) continue;
      files.push(p);
    }
  }
})(ROOT);

const SKIP_CALLEES =
  /^(t|i18n\.t|console\.\w+|invoke|listen|emit|require|import|Error|new Error|logger\.\w+|log\w*|warn|error|debug|info|commands\.\w+|localStorage\.\w+|sessionStorage\.\w+|setTimeout|matchMedia|window\.matchMedia|document\.\w+|querySelector\w*|addEventListener|removeEventListener|getSetting|updateSetting|RegExp|new RegExp|Intl\.\w+|toLocale\w+|cn|clsx|classNames|twMerge|cva)$/;
const SKIP_ATTRS = new Set([
  "className",
  "style",
  "type",
  "id",
  "name",
  "key",
  "href",
  "src",
  "role",
  "variant",
  "size",
  "side",
  "align",
  "as",
  "rel",
  "target",
  "method",
  "autoComplete",
  "inputMode",
  "lang",
  "dir",
  "mode",
  "value",
  "defaultValue",
  "htmlFor",
  "viewBox",
  "d",
  "fill",
  "stroke",
  "xmlns",
  "strokeLinecap",
  "strokeLinejoin",
  "fillRule",
  "clipRule",
  "transform",
  "points",
  "accept",
  "pattern",
  "form",
  "tabIndex",
  "draggable",
  "spellCheck",
  "color",
  "icon",
  "testId",
  "i18nKey",
  "ns",
  "keyPrefix",
  "tone",
  "kind",
  "status",
  "position",
  "direction",
  "orientation",
  "layout",
  "intent",
  "theme",
  "shape",
  "loading",
  "decoding",
  "sandbox",
  "allow",
  "crossOrigin",
  "referrerPolicy",
  "wrap",
  "step",
  "min",
  "max",
]);
const TEXT_ATTRS =
  /^(title|label|placeholder|description|aria-label|aria-description|alt|tooltip|hint|subtitle|heading|caption|message|text|helperText|emptyText|confirmLabel|cancelLabel|actionLabel|badge|summary|tip|note|header|content|prefix|suffix|ariaLabel|buttonLabel|emptyMessage|loadingLabel|srLabel)$/;

function looksLikeProse(s) {
  const v = s.trim();
  if (v.length < 2) return false;
  if (!/[A-Za-z]{2,}/.test(v)) return false;
  if (/^[a-z0-9_.:\-/#@]+$/.test(v)) return false; // identifiers, keys, paths
  if (/^[A-Z0-9_]+$/.test(v)) return false; // CONSTANTS
  if (/^(https?:|data:|blob:|mailto:|file:|\.|\/|#|--|\$\{)/.test(v))
    return false;
  if (/^[\w-]+(\s+[\w:-]+)*$/.test(v) && /[:\-]/.test(v) && !/[A-Z]/.test(v[0]))
    return false; // tailwind-ish
  if (
    /^(flex|grid|text-|bg-|border|rounded|px-|py-|p-|m-|w-|h-|gap-|items-|justify-)/.test(
      v,
    )
  )
    return false;
  if (/^[a-z]+([A-Z][a-z0-9]+)+$/.test(v)) return false; // camelCase
  if (
    /^(Ctrl|Alt|Shift|Cmd|Meta|Super|Escape|Enter|Space|Tab|Fn|Win)(\+\w+)*$/i.test(
      v,
    )
  )
    return false;
  // must have a space or start uppercase word to be prose
  return /\s/.test(v) || /^[A-Z][a-z]+/.test(v);
}

const out = [];
for (const file of files) {
  const src = fs.readFileSync(file, "utf8");
  const sf = ts.createSourceFile(
    file,
    src,
    ts.ScriptTarget.Latest,
    true,
    file.endsWith("x") ? ts.ScriptKind.TSX : ts.ScriptKind.TS,
  );
  const rel = path.relative(process.cwd(), file);
  const visit = (node) => {
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) return;
    if (ts.isCallExpression(node)) {
      const callee = node.expression.getText(sf).replace(/\?\./g, ".");
      if (
        SKIP_CALLEES.test(callee) ||
        /(^|\.)(t|log|warn|error|debug|info|trace)$/.test(callee)
      )
        return;
    }
    if (ts.isNewExpression(node) && /Error$/.test(node.expression.getText(sf)))
      return;
    if (ts.isJsxAttribute(node)) {
      const name = node.name.getText(sf);
      if (
        SKIP_ATTRS.has(name) ||
        name.startsWith("data-") ||
        /ClassName$|^on[A-Z]/.test(name)
      )
        return;
    }
    if (ts.isTypeNode(node) || ts.isLiteralTypeNode?.(node)) return;
    if (ts.isElementAccessExpression(node)) {
      visit(node.expression);
      return;
    }
    if (
      ts.isBinaryExpression(node) &&
      [
        ts.SyntaxKind.EqualsEqualsEqualsToken,
        ts.SyntaxKind.ExclamationEqualsEqualsToken,
        ts.SyntaxKind.EqualsEqualsToken,
      ].includes(node.operatorToken.kind)
    )
      return;
    if (ts.isCaseClause(node)) {
      node.statements.forEach(visit);
      return;
    }
    if (ts.isPropertyAssignment(node)) {
      const pn = node.name.getText(sf).replace(/["']/g, "");
      if (
        /^(id|key|value|type|kind|className|icon|variant|code|slug|provider|model|engine|url|href|path|event|mode|lang|locale|format|mime|shortcut|binding|setting|settingKey|i18nKey|labelKey|titleKey|descriptionKey|tKey)$/i.test(
          pn,
        )
      )
        return;
    }
    if (ts.isStringLiteral(node) || ts.isNoSubstitutionTemplateLiteral(node)) {
      if (looksLikeProse(node.text)) {
        const p = node.parent;
        // skip object keys
        if (ts.isPropertyAssignment(p) && p.name === node) return;
        const { line } = sf.getLineAndCharacterOfPosition(node.getStart(sf));
        let ctx = "str";
        if (
          ts.isJsxAttribute(p) ||
          (p && ts.isJsxExpression(p) && ts.isJsxAttribute(p.parent))
        )
          ctx =
            "attr:" +
            (ts.isJsxAttribute(p)
              ? p.name.getText(sf)
              : p.parent.name.getText(sf));
        else if (ts.isPropertyAssignment(p)) ctx = "prop:" + p.name.getText(sf);
        out.push({ file: rel, line: line + 1, ctx, text: node.text });
      }
    }
    if (ts.isTemplateExpression(node)) {
      const txt =
        node.head.text +
        node.templateSpans.map((s) => "${}" + s.literal.text).join("");
      if (
        looksLikeProse(txt.replace(/\$\{\}/g, "X")) &&
        /[A-Za-z]{3,}\s+[A-Za-z]{2,}/.test(txt)
      ) {
        const { line } = sf.getLineAndCharacterOfPosition(node.getStart(sf));
        out.push({ file: rel, line: line + 1, ctx: "tpl", text: txt });
      }
    }
    if (ts.isJsxText(node)) {
      const v = node.text.trim();
      if (/[A-Za-z]{2,}/.test(v)) {
        const { line } = sf.getLineAndCharacterOfPosition(node.getStart(sf));
        out.push({ file: rel, line: line + 1, ctx: "jsxtext", text: v });
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(sf);
}
fs.writeFileSync(
  process.env.TEMP + "/i18n-scan.json",
  JSON.stringify(out, null, 1),
);
const byFile = {};
for (const o of out) byFile[o.file] = (byFile[o.file] ?? 0) + 1;
console.log("total", out.length);
console.log(
  Object.entries(byFile)
    .sort((a, b) => b[1] - a[1])
    .map(([f, n]) => `${n}\t${f}`)
    .join("\n"),
);

import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { save, open } from "@tauri-apps/plugin-dialog";
import { Plus, Trash2, ChevronDown, Upload, Download } from "lucide-react";
import { commands } from "@/bindings";
import type { Capitalization, Replacement } from "@/bindings";
import { useSettings } from "../../hooks/useSettings";
import { Input } from "../ui/Input";
import { Button } from "../ui/Button";
import { ToggleSwitch } from "../ui/ToggleSwitch";

interface TextReplacementsProps {
  descriptionMode?: "inline" | "tooltip";
  grouped?: boolean;
}

// Local-only stable key so React can track rows across edits/removals without
// relying on array indices (which shift when a middle row is removed).
type Row = Replacement & { _key: string };

let keyCounter = 0;
const nextKey = () => `tr-${keyCounter++}`;

const withKey = (replacement: Replacement): Row => ({
  ...replacement,
  _key: nextKey(),
});

const stripKey = ({ _key, ...rest }: Row): Replacement => rest;

const emptyRule = (): Row =>
  withKey({
    search: "",
    replace: "",
    is_regex: false,
    enabled: true,
    trim_before: false,
    trim_after: false,
    capitalization: "none",
  });

// Coerce an arbitrary parsed object into a well-formed Replacement. Unknown
// fields are dropped. Notably, command-execution is NOT a per-rule field, so
// importing a file can never silently enable `[run]`.
const sanitizeRule = (raw: unknown): Replacement => {
  const r = (raw ?? {}) as Record<string, unknown>;
  const cap = r.capitalization;
  const capitalization: Capitalization =
    cap === "uppercase" || cap === "lowercase" || cap === "capitalize"
      ? cap
      : "none";
  return {
    search: typeof r.search === "string" ? r.search : "",
    replace: typeof r.replace === "string" ? r.replace : "",
    is_regex: Boolean(r.is_regex),
    enabled: r.enabled === undefined ? true : Boolean(r.enabled),
    trim_before: Boolean(r.trim_before),
    trim_after: Boolean(r.trim_after),
    capitalization,
  };
};

export const TextReplacements: React.FC<TextReplacementsProps> = React.memo(
  ({ descriptionMode = "tooltip", grouped = false }) => {
    const { t } = useTranslation();
    const { getSetting, updateSetting, isUpdating } = useSettings();

    const settingsRules = getSetting("text_replacements");
    const enabled = getSetting("replacements_enabled") ?? false;

    const [rows, setRows] = useState<Row[]>([]);
    const [openRows, setOpenRows] = useState<Record<string, boolean>>({});
    const initialized = useRef(false);
    const persistTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

    // Populate local rows once settings have loaded (settings start as null).
    useEffect(() => {
      if (!initialized.current && settingsRules) {
        setRows(settingsRules.map(withKey));
        initialized.current = true;
      }
    }, [settingsRules]);

    useEffect(() => {
      return () => {
        if (persistTimer.current) clearTimeout(persistTimer.current);
      };
    }, []);

    const persist = useCallback(
      (next: Row[]) => {
        updateSetting("text_replacements", next.map(stripKey));
      },
      [updateSetting],
    );

    // Debounced persistence for text inputs so each keystroke doesn't hit the
    // backend store; structural changes call `persist` directly.
    const persistDebounced = useCallback(
      (next: Row[]) => {
        if (persistTimer.current) clearTimeout(persistTimer.current);
        persistTimer.current = setTimeout(() => persist(next), 350);
      },
      [persist],
    );

    const updateRow = (
      index: number,
      patch: Partial<Replacement>,
      immediate = false,
    ) => {
      const next = rows.map((row, i) =>
        i === index ? { ...row, ...patch } : row,
      );
      setRows(next);
      if (immediate) {
        if (persistTimer.current) clearTimeout(persistTimer.current);
        persist(next);
      } else {
        persistDebounced(next);
      }
    };

    const addRow = () => {
      const row = emptyRule();
      const next = [...rows, row];
      setRows(next);
      setOpenRows((prev) => ({ ...prev, [row._key]: false }));
      persist(next);
    };

    const removeRow = (index: number) => {
      const next = rows.filter((_, i) => i !== index);
      setRows(next);
      if (persistTimer.current) clearTimeout(persistTimer.current);
      persist(next);
    };

    const toggleAdvanced = (key: string) => {
      setOpenRows((prev) => ({ ...prev, [key]: !prev[key] }));
    };

    const handleExport = async () => {
      try {
        // Native "save as" dialog so the user picks where the file goes.
        const path = await save({
          defaultPath: "text-replacements.json",
          filters: [{ name: "JSON", extensions: ["json"] }],
        });
        if (!path) return; // user cancelled
        const contents = JSON.stringify(rows.map(stripKey), null, 2);
        const result = await commands.exportTextReplacements(path, contents);
        if (result.status === "error") {
          toast.error(t("settings.advanced.textReplacements.exportError"));
          return;
        }
        toast.success(t("settings.advanced.textReplacements.exportSuccess"));
      } catch {
        toast.error(t("settings.advanced.textReplacements.exportError"));
      }
    };

    const handleImport = async () => {
      try {
        const selected = await open({
          multiple: false,
          filters: [{ name: "JSON", extensions: ["json"] }],
        });
        if (!selected || Array.isArray(selected)) return; // cancelled
        const result = await commands.importTextReplacements(selected);
        if (result.status === "error") {
          toast.error(t("settings.advanced.textReplacements.importError"));
          return;
        }
        const parsed = JSON.parse(result.data);
        if (!Array.isArray(parsed)) throw new Error("not an array");
        const next = parsed.map(sanitizeRule).map(withKey);
        setRows(next);
        if (persistTimer.current) clearTimeout(persistTimer.current);
        persist(next);
        toast.success(
          t("settings.advanced.textReplacements.importSuccess", {
            count: next.length,
          }),
        );
      } catch {
        toast.error(t("settings.advanced.textReplacements.importError"));
      }
    };

    const capitalizationOptions: Capitalization[] = [
      "none",
      "uppercase",
      "lowercase",
      "capitalize",
    ];

    const checkboxClass =
      "h-3.5 w-3.5 rounded border-hairline-strong text-ink focus:ring-ink/20 cursor-pointer";

    return (
      <>
        <ToggleSwitch
          checked={enabled}
          onChange={(value) => updateSetting("replacements_enabled", value)}
          isUpdating={isUpdating("replacements_enabled")}
          label={t("settings.advanced.textReplacements.title")}
          description={t("settings.advanced.textReplacements.description")}
          descriptionMode={descriptionMode}
          grouped={grouped}
        />

        {enabled && (
          <div className="px-4 pb-3 space-y-2">
            {rows.length === 0 && (
              <p className="text-sm text-muted py-2">
                {t("settings.advanced.textReplacements.empty")}
              </p>
            )}

            {rows.map((row, index) => {
              const open = openRows[row._key] ?? false;
              return (
                <div
                  key={row._key}
                  className={`rounded-xl border border-hairline bg-surface ${row.enabled ? "" : "opacity-60"}`}
                >
                  <div className="flex items-center gap-2 p-2">
                    <Input
                      type="text"
                      variant="compact"
                      className="flex-1 min-w-0"
                      value={row.search}
                      onChange={(e) =>
                        updateRow(index, { search: e.target.value })
                      }
                      placeholder={t(
                        "settings.advanced.textReplacements.searchPlaceholder",
                      )}
                      aria-label={t(
                        "settings.advanced.textReplacements.searchPlaceholder",
                      )}
                    />
                    <span className="text-muted-soft shrink-0">→</span>
                    <Input
                      type="text"
                      variant="compact"
                      className="flex-1 min-w-0"
                      value={row.replace}
                      onChange={(e) =>
                        updateRow(index, { replace: e.target.value })
                      }
                      placeholder={t(
                        "settings.advanced.textReplacements.replacePlaceholder",
                      )}
                      aria-label={t(
                        "settings.advanced.textReplacements.replacePlaceholder",
                      )}
                    />
                    <button
                      type="button"
                      aria-expanded={open}
                      aria-label={t(
                        "settings.advanced.textReplacements.advanced",
                      )}
                      onClick={() => toggleAdvanced(row._key)}
                      className="shrink-0 p-1.5 text-muted hover:text-ink transition-colors cursor-pointer"
                    >
                      <ChevronDown
                        className={`w-4 h-4 transition-transform duration-200 ${open ? "rotate-180" : ""}`}
                      />
                    </button>
                    <button
                      type="button"
                      aria-label={t(
                        "settings.advanced.textReplacements.remove",
                      )}
                      onClick={() => removeRow(index)}
                      className="shrink-0 p-1.5 text-muted hover:text-error transition-colors cursor-pointer"
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>

                  {open && (
                    <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-3 pb-3 pt-1 border-t border-hairline text-sm text-muted">
                      <label className="inline-flex items-center gap-1.5 cursor-pointer">
                        <input
                          type="checkbox"
                          className={checkboxClass}
                          checked={row.is_regex ?? false}
                          onChange={(e) =>
                            updateRow(
                              index,
                              { is_regex: e.target.checked },
                              true,
                            )
                          }
                        />
                        {t("settings.advanced.textReplacements.regex")}
                      </label>
                      <label className="inline-flex items-center gap-1.5 cursor-pointer">
                        <input
                          type="checkbox"
                          className={checkboxClass}
                          checked={row.trim_before ?? false}
                          onChange={(e) =>
                            updateRow(
                              index,
                              { trim_before: e.target.checked },
                              true,
                            )
                          }
                        />
                        {t("settings.advanced.textReplacements.trimBefore")}
                      </label>
                      <label className="inline-flex items-center gap-1.5 cursor-pointer">
                        <input
                          type="checkbox"
                          className={checkboxClass}
                          checked={row.trim_after ?? false}
                          onChange={(e) =>
                            updateRow(
                              index,
                              { trim_after: e.target.checked },
                              true,
                            )
                          }
                        />
                        {t("settings.advanced.textReplacements.trimAfter")}
                      </label>
                      <label className="inline-flex items-center gap-1.5">
                        <span>
                          {t(
                            "settings.advanced.textReplacements.capitalization.label",
                          )}
                        </span>
                        <select
                          className="text-sm bg-surface border border-hairline-strong rounded-lg px-2 py-1 text-ink focus:outline-none focus:border-ink cursor-pointer"
                          value={row.capitalization ?? "none"}
                          onChange={(e) =>
                            updateRow(
                              index,
                              {
                                capitalization: e.target
                                  .value as Capitalization,
                              },
                              true,
                            )
                          }
                        >
                          {capitalizationOptions.map((option) => (
                            <option key={option} value={option}>
                              {t(
                                `settings.advanced.textReplacements.capitalization.${option}`,
                              )}
                            </option>
                          ))}
                        </select>
                      </label>
                      <label className="inline-flex items-center gap-1.5 cursor-pointer ml-auto">
                        <input
                          type="checkbox"
                          className={checkboxClass}
                          checked={row.enabled ?? true}
                          onChange={(e) =>
                            updateRow(
                              index,
                              { enabled: e.target.checked },
                              true,
                            )
                          }
                        />
                        {t("settings.advanced.textReplacements.enabledLabel")}
                      </label>
                    </div>
                  )}
                </div>
              );
            })}

            <p className="text-xs text-muted-soft pt-1">
              {t("settings.advanced.textReplacements.magicHint")}
            </p>

            <div className="flex items-center gap-2 pt-1">
              <Button variant="secondary" size="sm" onClick={addRow}>
                <Plus className="w-3.5 h-3.5" />
                {t("settings.advanced.textReplacements.addRule")}
              </Button>
              <div className="ml-auto flex items-center gap-2">
                <Button variant="ghost" size="sm" onClick={handleImport}>
                  <Upload className="w-3.5 h-3.5" />
                  {t("settings.advanced.textReplacements.import")}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={handleExport}
                  disabled={rows.length === 0}
                >
                  <Download className="w-3.5 h-3.5" />
                  {t("settings.advanced.textReplacements.export")}
                </Button>
              </div>
            </div>
          </div>
        )}
      </>
    );
  },
);

TextReplacements.displayName = "TextReplacements";

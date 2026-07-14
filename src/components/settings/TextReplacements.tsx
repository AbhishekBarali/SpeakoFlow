import React, { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { save, open } from "@tauri-apps/plugin-dialog";
import {
  ArrowRight,
  Plus,
  Trash2,
  ChevronDown,
  Upload,
  Download,
} from "lucide-react";
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
  ({ descriptionMode = "inline", grouped = false }) => {
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

    const handleEnabledChange = (value: boolean) => {
      void updateSetting("replacements_enabled", value);
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
          onChange={handleEnabledChange}
          isUpdating={isUpdating("replacements_enabled")}
          label={t("settings.advanced.textReplacements.title")}
          description={t("settings.advanced.textReplacements.description")}
          info={t("settings.advanced.textReplacements.magicHint")}
          descriptionMode={grouped ? "tooltip" : descriptionMode}
          grouped={grouped}
        />

        {enabled && (
          <section
            aria-label={t("settings.advanced.textReplacements.rulesTitle")}
            className="px-4 py-3"
          >
            <div className="flex items-center gap-1.5">
              <Button
                variant="primary-soft"
                size="sm"
                onClick={addRow}
                disabled={isUpdating("text_replacements")}
              >
                <Plus className="h-3.5 w-3.5" aria-hidden="true" />
                {t("settings.advanced.textReplacements.addRule")}
              </Button>
              <div className="ms-auto flex items-center gap-0.5">
                <Button variant="ghost" size="sm" onClick={handleImport}>
                  <Upload className="h-3.5 w-3.5" aria-hidden="true" />
                  {t("settings.advanced.textReplacements.import")}
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  onClick={handleExport}
                  disabled={rows.length === 0}
                >
                  <Download className="h-3.5 w-3.5" aria-hidden="true" />
                  {t("settings.advanced.textReplacements.export")}
                </Button>
              </div>
            </div>

            {rows.length > 0 && (
              <div className="mt-2.5 space-y-2">
                {rows.map((row, index) => {
                  const open = openRows[row._key] ?? false;
                  return (
                    <div
                      key={row._key}
                      className={`overflow-hidden rounded-xl bg-surface-strong ring-1 ring-inset ring-hairline ${row.enabled ? "" : "opacity-60"}`}
                    >
                      <div className="flex items-end gap-2 p-2.5">
                        <label className="min-w-0 flex-1 space-y-1">
                          <span className="block text-[11px] font-medium leading-none text-muted">
                            {t(
                              "settings.advanced.textReplacements.searchLabel",
                            )}
                          </span>
                          <Input
                            type="text"
                            variant="compact"
                            className="w-full"
                            value={row.search}
                            onChange={(e) =>
                              updateRow(index, { search: e.target.value })
                            }
                            placeholder={t(
                              "settings.advanced.textReplacements.searchPlaceholder",
                            )}
                          />
                        </label>
                        <ArrowRight
                          className="mb-2 h-4 w-4 shrink-0 text-muted-soft"
                          aria-hidden="true"
                        />
                        <label className="min-w-0 flex-1 space-y-1">
                          <span className="block text-[11px] font-medium leading-none text-muted">
                            {t(
                              "settings.advanced.textReplacements.replaceLabel",
                            )}
                          </span>
                          <Input
                            type="text"
                            variant="compact"
                            className="w-full"
                            value={row.replace}
                            onChange={(e) =>
                              updateRow(index, { replace: e.target.value })
                            }
                            placeholder={t(
                              "settings.advanced.textReplacements.replacePlaceholder",
                            )}
                          />
                        </label>
                        <button
                          type="button"
                          aria-expanded={open}
                          aria-label={t(
                            "settings.advanced.textReplacements.advanced",
                          )}
                          onClick={() => toggleAdvanced(row._key)}
                          className="mb-0.5 shrink-0 cursor-pointer rounded-md p-1.5 text-muted transition-colors hover:bg-ink/5 hover:text-ink focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent/60"
                        >
                          <ChevronDown
                            className={`h-4 w-4 transition-transform duration-200 ${open ? "rotate-180" : ""}`}
                          />
                        </button>
                        <button
                          type="button"
                          aria-label={t(
                            "settings.advanced.textReplacements.remove",
                          )}
                          onClick={() => removeRow(index)}
                          className="mb-0.5 shrink-0 cursor-pointer rounded-md p-1.5 text-muted transition-colors hover:bg-error/10 hover:text-error focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-error/60"
                        >
                          <Trash2 className="h-4 w-4" />
                        </button>
                      </div>

                      {open && (
                        <div className="flex flex-wrap items-center gap-x-4 gap-y-2 border-t border-hairline px-3 py-2.5 text-xs text-muted">
                          <label className="inline-flex cursor-pointer items-center gap-1.5">
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
                          <label className="inline-flex cursor-pointer items-center gap-1.5">
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
                          <label className="inline-flex cursor-pointer items-center gap-1.5">
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
                              className="cursor-pointer rounded-lg border border-hairline-strong bg-surface px-2 py-1 text-xs text-ink focus:border-accent focus:outline-none"
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
                          <label className="ms-auto inline-flex cursor-pointer items-center gap-1.5">
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
                            {t(
                              "settings.advanced.textReplacements.enabledLabel",
                            )}
                          </label>
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </section>
        )}
      </>
    );
  },
);

TextReplacements.displayName = "TextReplacements";

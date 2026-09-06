import { useEffect, useId, useRef, useState } from "react";
import { Check, ChevronDown } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AssistantCharacter } from "@/bindings";

function Avatar({ profile }: { profile: AssistantCharacter | null }) {
  return profile?.avatar ? (
    <img className="assistant-character-avatar" src={profile.avatar} alt="" />
  ) : (
    <span className="assistant-character-avatar" aria-hidden="true">
      {profile?.kind === "cat"
        ? "🐱"
        : (profile?.name.trim()[0] ?? "?").toUpperCase()}
    </span>
  );
}

export function AssistantProfilePicker({
  profiles,
  activeId,
  onSelect,
  conversation = false,
}: {
  profiles: AssistantCharacter[];
  activeId: string;
  onSelect: (id: string) => Promise<void>;
  conversation?: boolean;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [pending, setPending] = useState(false);
  const [failed, setFailed] = useState(false);
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const popup = useId();
  const active = profiles.find((p) => p.id === activeId) ?? profiles[0] ?? null;

  useEffect(() => {
    if (!open) return;
    const outside = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    window.addEventListener("pointerdown", outside);
    return () => window.removeEventListener("pointerdown", outside);
  }, [open]);

  const select = async (id: string) => {
    setPending(true);
    setFailed(false);
    try {
      await onSelect(id);
      setOpen(false);
      trigger.current?.focus();
    } catch {
      setFailed(true);
    } finally {
      setPending(false);
    }
  };

  return (
    <div
      className={`assistant-character${conversation ? " voice-profile" : ""}`}
      ref={root}
      onKeyDown={(event) => {
        if (event.key === "Escape" && open) {
          event.preventDefault();
          event.stopPropagation();
          setOpen(false);
          trigger.current?.focus();
        }
      }}
      onBlur={(event) => {
        if (!event.currentTarget.contains(event.relatedTarget)) setOpen(false);
      }}
    >
      <button
        ref={trigger}
        type="button"
        className="assistant-character-switch"
        aria-expanded={open}
        aria-controls={popup}
        aria-label={t("assistant.conversation.chooseProfile", {
          name: active?.name ?? t("assistant.title"),
        })}
        onClick={() => {
          setOpen(!open);
          setFailed(false);
        }}
      >
        <Avatar profile={active} />
        <span className="assistant-character-name">
          {active?.name ?? t("assistant.title")}
        </span>
        <ChevronDown size={13} className="as-chevron" />
      </button>
      {open && (
        <div
          id={popup}
          className="assistant-character-menu"
          role="group"
          aria-label={t("assistant.character.switch")}
        >
          <div className="assistant-profile-heading">
            <span>{t("assistant.conversation.profiles")}</span>
            {conversation && <p>{t("assistant.conversation.profileHint")}</p>}
          </div>
          {profiles.map((profile) => (
            <button
              key={profile.id}
              type="button"
              className={`assistant-character-item${profile.id === activeId ? " active" : ""}`}
              disabled={pending || (conversation && profile.kind === "cat")}
              title={
                conversation && profile.kind === "cat"
                  ? t("assistant.conversation.chatOnlyProfile")
                  : undefined
              }
              aria-pressed={profile.id === activeId}
              onClick={() => void select(profile.id)}
            >
              <Avatar profile={profile} />
              <span className="assistant-character-name">{profile.name}</span>
              {profile.id === activeId && (
                <Check size={14} className="as-check" />
              )}
            </button>
          ))}
          {failed && (
            <p className="assistant-profile-error" role="alert">
              {t("assistant.conversation.profileError")}
            </p>
          )}
        </div>
      )}
    </div>
  );
}

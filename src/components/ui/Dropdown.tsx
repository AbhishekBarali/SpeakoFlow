import React, { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";

export interface DropdownOption {
  value: string;
  label: string;
  disabled?: boolean;
}

interface DropdownProps {
  options: DropdownOption[];
  className?: string;
  selectedValue: string | null;
  onSelect: (value: string) => void;
  placeholder?: string;
  disabled?: boolean;
  onRefresh?: () => void;
}

export const Dropdown: React.FC<DropdownProps> = ({
  options,
  selectedValue,
  onSelect,
  className = "",
  placeholder = "Select an option...",
  disabled = false,
  onRefresh,
}) => {
  const { t } = useTranslation();
  const [isOpen, setIsOpen] = useState(false);
  const dropdownRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const handleClickOutside = (event: MouseEvent) => {
      if (
        dropdownRef.current &&
        !dropdownRef.current.contains(event.target as Node)
      ) {
        setIsOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClickOutside);
    return () => document.removeEventListener("mousedown", handleClickOutside);
  }, []);

  const selectedOption = options.find(
    (option) => option.value === selectedValue,
  );

  const handleSelect = (value: string) => {
    onSelect(value);
    setIsOpen(false);
  };

  const handleToggle = () => {
    if (disabled) return;
    if (!isOpen && onRefresh) onRefresh();
    setIsOpen(!isOpen);
  };

  return (
    <div className={`relative ${className}`} ref={dropdownRef}>
      <button
        type="button"
        className={`w-full px-3 py-2 text-sm bg-surface border border-hairline-strong rounded-lg min-w-[200px] text-start flex items-center justify-between transition-colors duration-150 ${
          disabled
            ? "opacity-50 cursor-not-allowed"
            : "hover:border-ink/40 cursor-pointer"
        }`}
        onClick={handleToggle}
        disabled={disabled}
        title={selectedOption?.label || placeholder}
      >
        <span className="truncate min-w-0">
          {selectedOption?.label || placeholder}
        </span>
        <svg
          className={`w-4 h-4 ms-2 transition-transform duration-200 ${isOpen ? "transform rotate-180" : ""}`}
          fill="none"
          stroke="currentColor"
          viewBox="0 0 24 24"
        >
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={2}
            d="M19 9l-7 7-7-7"
          />
        </svg>
      </button>
      {isOpen && !disabled && (
        <div className="absolute top-full left-0 right-0 mt-1.5 glass-menu border border-hairline rounded-xl shadow-lg z-50 max-h-60 overflow-y-auto overflow-x-hidden p-1">
          {options.length === 0 ? (
            <div className="px-2.5 py-1.5 text-sm text-muted">
              {t("common.noOptionsFound")}
            </div>
          ) : (
            options.map((option) => (
              <button
                key={option.value}
                type="button"
                className={`flex items-center w-full px-2.5 py-1.5 text-sm text-start rounded-lg overflow-hidden hover:bg-surface-strong transition-colors duration-150 ${
                  selectedValue === option.value
                    ? "bg-surface-strong font-medium"
                    : ""
                } ${option.disabled ? "opacity-50 cursor-not-allowed" : ""}`}
                onClick={() => handleSelect(option.value)}
                disabled={option.disabled}
                title={option.label}
              >
                <span className="truncate min-w-0 flex-1">{option.label}</span>
              </button>
            ))
          )}
        </div>
      )}
    </div>
  );
};

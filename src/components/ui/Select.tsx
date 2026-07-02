import React from "react";
import SelectComponent from "react-select";
import CreatableSelect from "react-select/creatable";
import type {
  ActionMeta,
  Props as ReactSelectProps,
  SingleValue,
  StylesConfig,
} from "react-select";

export type SelectOption = {
  value: string;
  label: string;
  isDisabled?: boolean;
};

type BaseProps = {
  value: string | null;
  options: SelectOption[];
  placeholder?: string;
  disabled?: boolean;
  isLoading?: boolean;
  isClearable?: boolean;
  onChange: (value: string | null, action: ActionMeta<SelectOption>) => void;
  onBlur?: () => void;
  className?: string;
  formatCreateLabel?: (input: string) => string;
};

type CreatableProps = {
  isCreatable: true;
  onCreateOption: (value: string) => void;
};

type NonCreatableProps = {
  isCreatable?: false;
  onCreateOption?: never;
};

export type SelectProps = BaseProps & (CreatableProps | NonCreatableProps);

const surfaceBg = "var(--color-surface)";
const hoverBg = "var(--color-surface-strong)";
const neutralBorder = "var(--color-hairline-strong)";
const accentBorder = "var(--color-accent)";

const selectStyles: StylesConfig<SelectOption, false> = {
  control: (base, state) => ({
    ...base,
    minHeight: 36,
    borderRadius: 8,
    borderColor: state.isFocused ? accentBorder : neutralBorder,
    boxShadow: state.isFocused
      ? "0 0 0 3px color-mix(in srgb, var(--color-accent) 20%, transparent)"
      : "none",
    backgroundColor: surfaceBg,
    fontSize: "0.875rem",
    color: "var(--color-text)",
    transition: "border-color 150ms ease, box-shadow 150ms ease",
    ":hover": {
      borderColor: state.isFocused ? accentBorder : neutralBorder,
    },
  }),
  valueContainer: (base) => ({
    ...base,
    paddingInline: 12,
    paddingBlock: 6,
  }),
  input: (base) => ({
    ...base,
    color: "var(--color-text)",
  }),
  singleValue: (base) => ({
    ...base,
    color: "var(--color-text)",
  }),
  dropdownIndicator: (base, state) => ({
    ...base,
    color: state.isFocused ? "var(--color-ink)" : "var(--color-muted-soft)",
    ":hover": {
      color: "var(--color-ink)",
    },
  }),
  clearIndicator: (base) => ({
    ...base,
    color: "var(--color-muted-soft)",
    ":hover": {
      color: "var(--color-ink)",
    },
  }),
  menu: (provided) => ({
    ...provided,
    zIndex: 30,
    borderRadius: 12,
    overflow: "hidden",
    backgroundColor: surfaceBg,
    color: "var(--color-text)",
    border: "1px solid var(--color-hairline)",
    boxShadow: "0 10px 30px rgba(0, 0, 0, 0.12)",
  }),
  option: (base, state) => ({
    ...base,
    backgroundColor:
      state.isSelected || state.isFocused ? hoverBg : "transparent",
    color: "var(--color-text)",
    fontWeight: state.isSelected ? 500 : 400,
    cursor: state.isDisabled ? "not-allowed" : base.cursor,
    opacity: state.isDisabled ? 0.5 : 1,
  }),
  placeholder: (base) => ({
    ...base,
    color: "var(--color-muted-soft)",
  }),
};

export const Select: React.FC<SelectProps> = React.memo(
  ({
    value,
    options,
    placeholder,
    disabled,
    isLoading,
    isClearable = true,
    onChange,
    onBlur,
    className = "",
    isCreatable,
    formatCreateLabel,
    onCreateOption,
  }) => {
    const selectValue = React.useMemo(() => {
      if (!value) return null;
      const existing = options.find((option) => option.value === value);
      if (existing) return existing;
      return { value, label: value, isDisabled: false };
    }, [value, options]);

    const handleChange = (
      option: SingleValue<SelectOption>,
      action: ActionMeta<SelectOption>,
    ) => {
      onChange(option?.value ?? null, action);
    };

    const sharedProps: Partial<ReactSelectProps<SelectOption, false>> = {
      className,
      classNamePrefix: "app-select",
      value: selectValue,
      options,
      onChange: handleChange,
      placeholder,
      isDisabled: disabled,
      isLoading,
      onBlur,
      isClearable,
      styles: selectStyles,
    };

    if (isCreatable) {
      return (
        <CreatableSelect<SelectOption, false>
          {...sharedProps}
          onCreateOption={onCreateOption}
          formatCreateLabel={formatCreateLabel}
        />
      );
    }

    return <SelectComponent<SelectOption, false> {...sharedProps} />;
  },
);

Select.displayName = "Select";

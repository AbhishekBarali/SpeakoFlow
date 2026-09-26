import React from "react";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?:
    | "primary"
    | "accent"
    | "primary-soft"
    | "secondary"
    | "danger"
    | "danger-ghost"
    | "ghost";
  size?: "sm" | "md" | "lg";
}

/**
 * One button vocabulary for the whole window.
 *
 * `primary` is ink — the pen-dark button a reference app uses for "Add new" —
 * so the brand teal stays free to mean "this is on / this is selected".
 * `accent` is the teal fill, for the rare action that should read as brand.
 */
export const Button: React.FC<ButtonProps> = ({
  children,
  className = "",
  variant = "primary",
  size = "md",
  ...props
}) => {
  // The border colour lives in each variant, never in the base: a base
  // `border-transparent` is emitted later in the stylesheet than
  // `border-hairline-strong`, so it silently won and every secondary button
  // rendered with no visible edge.
  const baseClasses =
    "inline-flex shrink-0 items-center justify-center gap-1.5 font-medium whitespace-nowrap border rounded-lg transition-[background-color,border-color,opacity,box-shadow,transform] duration-150 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 active:scale-[0.98] disabled:opacity-40 disabled:cursor-not-allowed disabled:active:scale-100 cursor-pointer";

  const variantClasses = {
    primary:
      "bg-ink text-on-ink border-transparent shadow-[0_1px_2px_rgba(27,26,24,0.18)] hover:bg-ink-soft",
    accent:
      "bg-accent text-on-primary border-transparent hover:bg-accent-strong",
    "primary-soft":
      "bg-surface-strong text-ink border-transparent hover:bg-hairline-strong/70",
    secondary:
      "bg-surface text-ink border-hairline-strong shadow-[0_1px_1px_rgba(27,26,24,0.04)] hover:bg-surface-strong",
    danger: "text-white bg-error border-transparent hover:opacity-90",
    "danger-ghost":
      "text-error border-transparent hover:bg-error/10 focus-visible:ring-error/30",
    ghost: "text-ink border-transparent hover:bg-ink/[0.05]",
  };

  const sizeClasses = {
    sm: "h-8 px-3 text-[0.8125rem]",
    md: "h-9 px-4 text-sm",
    lg: "h-10 px-5 text-[0.9375rem]",
  };

  return (
    <button
      className={`${baseClasses} ${variantClasses[variant]} ${sizeClasses[size]} ${className}`}
      {...props}
    >
      {children}
    </button>
  );
};

import React from "react";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?:
    | "primary"
    | "primary-soft"
    | "secondary"
    | "danger"
    | "danger-ghost"
    | "ghost";
  size?: "sm" | "md" | "lg";
}

export const Button: React.FC<ButtonProps> = ({
  children,
  className = "",
  variant = "primary",
  size = "md",
  ...props
}) => {
  // Quiet, rectangular geometry — accent carries the primary action, every
  // other variant stays neutral. No pills.
  const baseClasses =
    "inline-flex items-center justify-center gap-2 font-medium border border-transparent rounded-lg transition-[background-color,border-color,opacity,box-shadow,transform] duration-150 focus:outline-none focus-visible:ring-2 focus-visible:ring-accent/40 active:scale-[0.98] disabled:opacity-40 disabled:cursor-not-allowed disabled:active:scale-100 cursor-pointer";

  const variantClasses = {
    primary: "bg-accent text-on-primary hover:bg-accent-strong",
    "primary-soft":
      "bg-surface-strong text-ink hover:bg-hairline-strong/70 border-hairline",
    secondary:
      "bg-surface text-ink border-hairline-strong hover:bg-surface-strong",
    danger: "text-white bg-error hover:opacity-90",
    "danger-ghost":
      "text-error border-transparent hover:bg-error/10 focus-visible:ring-error/30",
    ghost: "text-ink border-transparent hover:bg-surface-strong",
  };

  const sizeClasses = {
    sm: "px-2.5 py-1.5 text-xs",
    md: "px-4 py-2 text-[13px]",
    lg: "px-5 py-2.5 text-sm",
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

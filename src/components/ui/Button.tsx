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
  // Pill geometry, ink-led palette. Boldness lives in the gradient orbs
  // elsewhere — buttons stay quiet and disciplined.
  const baseClasses =
    "inline-flex items-center justify-center gap-2 font-medium border border-transparent rounded-full transition-[background-color,border-color,opacity,box-shadow] duration-150 focus:outline-none focus-visible:ring-2 focus-visible:ring-ink/25 disabled:opacity-40 disabled:cursor-not-allowed cursor-pointer";

  const variantClasses = {
    primary:
      "bg-background-ui text-on-primary hover:opacity-90 active:opacity-100",
    "primary-soft":
      "bg-surface-strong text-ink hover:bg-hairline-strong/60 border-hairline",
    secondary:
      "bg-surface text-ink border-hairline-strong hover:bg-surface-strong hover:border-ink/30",
    danger: "text-white bg-error hover:opacity-90",
    "danger-ghost":
      "text-error border-transparent hover:bg-error/10 focus-visible:ring-error/30",
    ghost: "text-ink border-transparent hover:bg-surface-strong",
  };

  const sizeClasses = {
    sm: "px-3 py-1.5 text-xs",
    md: "px-5 py-2 text-sm",
    lg: "px-6 py-2.5 text-base",
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

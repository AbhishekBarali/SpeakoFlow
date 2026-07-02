import React from "react";

interface InputProps extends React.InputHTMLAttributes<HTMLInputElement> {
  variant?: "default" | "compact";
}

export const Input: React.FC<InputProps> = ({
  className = "",
  variant = "default",
  disabled,
  ...props
}) => {
  const baseClasses =
    "text-sm bg-surface border border-hairline-strong rounded-lg text-ink placeholder:text-muted-soft text-start transition-colors duration-150";

  const interactiveClasses = disabled
    ? "opacity-60 cursor-not-allowed bg-surface-strong border-hairline"
    : "hover:border-hairline-strong focus:outline-none focus:border-accent focus:ring-2 focus:ring-accent/20";

  const variantClasses = {
    default: "px-3 py-2",
    compact: "px-2.5 py-1.5",
  } as const;

  return (
    <input
      className={`${baseClasses} ${variantClasses[variant]} ${interactiveClasses} ${className}`}
      disabled={disabled}
      {...props}
    />
  );
};

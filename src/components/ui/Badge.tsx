import React from "react";

interface BadgeProps {
  children: React.ReactNode;
  variant?: "primary" | "success" | "secondary" | "active" | "outline";
  className?: string;
}

const Badge: React.FC<BadgeProps> = ({
  children,
  variant = "primary",
  className = "",
}) => {
  const variantClasses = {
    primary: "bg-surface-strong text-ink",
    success: "bg-success/15 text-success",
    secondary: "bg-surface-strong text-muted",
    // Solid ink pill — the current selection. Reads clearly even on a
    // surface-strong active card (where the old bg-surface-strong badge vanished).
    active: "bg-ink text-on-primary",
    // Quiet outline — a suggestion ("Recommended"), distinct from the solid
    // active state and legible on any surface.
    outline: "border border-hairline-strong text-muted bg-transparent",
  };

  return (
    <span
      className={`inline-flex items-center px-3 py-1 rounded-full text-xs font-medium ${variantClasses[variant]} ${className}`}
    >
      {children}
    </span>
  );
};

export default Badge;

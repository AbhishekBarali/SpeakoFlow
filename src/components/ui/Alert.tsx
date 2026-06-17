import React from "react";
import { AlertCircle, AlertTriangle, Info, CheckCircle } from "lucide-react";

type AlertVariant = "error" | "warning" | "info" | "success";

interface AlertProps {
  variant?: AlertVariant;
  /** When true, removes rounded corners for use inside containers */
  contained?: boolean;
  children: React.ReactNode;
  className?: string;
}

const variantStyles: Record<
  AlertVariant,
  { container: string; icon: string; text: string }
> = {
  error: {
    container: "bg-error/10",
    icon: "text-error",
    text: "text-red-700 dark:text-red-300",
  },
  warning: {
    container: "bg-amber-500/10",
    icon: "text-amber-600 dark:text-amber-500",
    text: "text-amber-700 dark:text-amber-300",
  },
  info: {
    container: "bg-sky-500/10",
    icon: "text-sky-600 dark:text-sky-500",
    text: "text-sky-700 dark:text-sky-300",
  },
  success: {
    container: "bg-success/10",
    icon: "text-success",
    text: "text-green-700 dark:text-green-300",
  },
};

const variantIcons: Record<AlertVariant, React.ElementType> = {
  error: AlertCircle,
  warning: AlertTriangle,
  info: Info,
  success: CheckCircle,
};

export const Alert: React.FC<AlertProps> = ({
  variant = "error",
  contained = false,
  children,
  className = "",
}) => {
  const styles = variantStyles[variant];
  const Icon = variantIcons[variant];

  return (
    <div
      className={`flex items-start gap-3 p-4 ${styles.container} ${contained ? "" : "rounded-lg"} ${className}`}
    >
      <Icon className={`w-5 h-5 shrink-0 mt-0.5 ${styles.icon}`} />
      <p className={`text-sm ${styles.text}`}>{children}</p>
    </div>
  );
};

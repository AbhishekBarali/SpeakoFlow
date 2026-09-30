export const formatModelSize = (sizeMb: number | null | undefined): string => {
  if (!sizeMb || !Number.isFinite(sizeMb) || sizeMb <= 0) {
    return "Unknown size";
  }

  if (sizeMb >= 1024) {
    const sizeGb = sizeMb / 1024;
    const formatter = new Intl.NumberFormat(undefined, {
      minimumFractionDigits: sizeGb >= 10 ? 0 : 1,
      maximumFractionDigits: sizeGb >= 10 ? 0 : 1,
    });
    return `${formatter.format(sizeGb)} GB`;
  }

  const formatter = new Intl.NumberFormat(undefined, {
    minimumFractionDigits: sizeMb >= 100 ? 0 : 1,
    maximumFractionDigits: sizeMb >= 100 ? 0 : 1,
  });

  return `${formatter.format(sizeMb)} MB`;
};

/** A raw byte count as a compact, locale-aware MB/GB string. */
export const formatBytes = (bytes: number): string => {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 MB";
  const mb = bytes / (1024 * 1024);
  if (mb >= 1024) {
    const gb = mb / 1024;
    const formatter = new Intl.NumberFormat(undefined, {
      minimumFractionDigits: gb >= 10 ? 0 : 1,
      maximumFractionDigits: gb >= 10 ? 0 : 1,
    });
    return `${formatter.format(gb)} GB`;
  }
  const formatter = new Intl.NumberFormat(undefined, {
    minimumFractionDigits: mb >= 100 ? 0 : 1,
    maximumFractionDigits: mb >= 100 ? 0 : 1,
  });
  return `${formatter.format(mb)} MB`;
};
